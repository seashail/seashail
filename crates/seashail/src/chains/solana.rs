use base64::Engine as _;
use bincode::Options as _;
use eyre::Context as _;
use reqwest::Client;
use serde_json::Value;
use solana_address::Address;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{
    RpcAccountInfoConfig, RpcProgramAccountsConfig, UiAccountEncoding,
};
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_client::rpc_response::RpcSimulateTransactionResult;
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{
    account::Account,
    hash::Hash,
    message::VersionedMessage,
    program_pack::Pack as _,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer as _,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_instruction;
use spl_associated_token_account::get_associated_token_address;
use spl_token::state::{Account as SplAccount, Mint};
use std::{str::FromStr as _, time::Duration};

use crate::retry::{try_all_with_backoff, BackoffConfig};

const MAX_REMOTE_TX_BYTES: u64 = 2 * 1024 * 1024;

const fn compute_budget_program_id() -> solana_sdk::pubkey::Pubkey {
    // Base58("ComputeBudget111111111111111111111111111111")
    solana_sdk::pubkey::Pubkey::new_from_array([
        3, 6, 70, 111, 229, 33, 23, 50, 255, 236, 173, 186, 114, 195, 155, 231, 188, 140, 229, 187,
        197, 247, 18, 107, 44, 67, 155, 58, 64, 0, 0, 0,
    ])
}

fn compute_budget_set_compute_unit_limit(units: u32) -> solana_sdk::instruction::Instruction {
    let mut data = Vec::with_capacity(1 + 4);
    data.push(2); // SetComputeUnitLimit
    data.extend_from_slice(&units.to_le_bytes());
    solana_sdk::instruction::Instruction {
        program_id: compute_budget_program_id(),
        accounts: vec![],
        data,
    }
}

fn compute_budget_set_compute_unit_price(
    micro_lamports: u64,
) -> solana_sdk::instruction::Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(3); // SetComputeUnitPrice
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    solana_sdk::instruction::Instruction {
        program_id: compute_budget_program_id(),
        accounts: vec![],
        data,
    }
}

// Known Jupiter program IDs for provider allowlisting.
const JUPITER_PROGRAMS: [&str; 3] = [
    "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
    "JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB",
    "JUP2jxvQffg8W4Jw8dYpQ8eQJ8v1s5p8yL6kD3m1j7d",
];

fn program_ids<'a>(
    msg: &'a VersionedMessage,
    keys: &'a [Address],
) -> eyre::Result<Vec<&'a Address>> {
    let mut out = vec![];
    match msg {
        VersionedMessage::Legacy(m) => {
            for ix in &m.instructions {
                let i = ix.program_id_index as usize;
                let pid = keys.get(i).ok_or_else(|| {
                    eyre::eyre!("invalid instruction: program_id_index out of bounds")
                })?;
                out.push(pid);
            }
        }
        VersionedMessage::V0(m) => {
            for ix in &m.instructions {
                let i = ix.program_id_index as usize;
                let pid = keys.get(i).ok_or_else(|| {
                    eyre::eyre!("invalid instruction: program_id_index out of bounds")
                })?;
                out.push(pid);
            }
        }
    }
    Ok(out)
}

fn allow_insecure_http() -> bool {
    std::env::var("SEASHAIL_ALLOW_INSECURE_HTTP")
        .ok()
        .is_some_and(|v| {
            matches!(
                v.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
}

fn is_loopback_http(url: &str) -> bool {
    fn host_prefix_ok(s: &str, prefix: &str) -> bool {
        if !s.starts_with(prefix) {
            return false;
        }
        matches!(s.as_bytes().get(prefix.len()), None | Some(b':' | b'/'))
    }
    let u = url.trim();
    host_prefix_ok(u, "http://127.0.0.1")
        || host_prefix_ok(u, "http://localhost")
        || host_prefix_ok(u, "http://[::1]")
}

#[derive(Debug, Clone)]
pub struct SolanaChain {
    pub rpc_url: String,
    pub fallback_rpc_urls: Vec<String>,
    pub jupiter_base_url: String,
    pub jupiter_api_key: Option<String>,
    pub default_compute_unit_limit: Option<u32>,
    pub default_compute_unit_price_micro_lamports: Option<u64>,
}

impl SolanaChain {
    pub fn new_with_fallbacks(
        rpc_url: &str,
        fallback_rpc_urls: &[String],
        jupiter_base_url: &str,
        jupiter_api_key: Option<&str>,
        default_compute_unit_limit: Option<u32>,
        default_compute_unit_price_micro_lamports: Option<u64>,
    ) -> Self {
        Self {
            rpc_url: rpc_url.to_owned(),
            fallback_rpc_urls: fallback_rpc_urls.to_vec(),
            jupiter_base_url: jupiter_base_url.to_owned(),
            jupiter_api_key: jupiter_api_key.map(str::to_owned),
            default_compute_unit_limit,
            default_compute_unit_price_micro_lamports,
        }
    }

    fn with_compute_budget_defaults(
        &self,
        mut instructions: Vec<solana_sdk::instruction::Instruction>,
    ) -> Vec<solana_sdk::instruction::Instruction> {
        let has_compute_budget = instructions
            .iter()
            .any(|ix| ix.program_id == compute_budget_program_id());
        if has_compute_budget {
            return instructions;
        }

        let limit = self.default_compute_unit_limit;
        let price = self.default_compute_unit_price_micro_lamports;
        if limit.is_none() && price.is_none() {
            return instructions;
        }

        let mut prefix: Vec<solana_sdk::instruction::Instruction> = vec![];
        if let Some(l) = limit {
            if l > 0 {
                prefix.push(compute_budget_set_compute_unit_limit(l));
            }
        }
        if let Some(p) = price {
            if p > 0 {
                prefix.push(compute_budget_set_compute_unit_price(p));
            }
        }
        if prefix.is_empty() {
            return instructions;
        }
        prefix.append(&mut instructions);
        prefix
    }

    fn all_rpc_urls(&self) -> Vec<String> {
        let mut urls = Vec::with_capacity(1 + self.fallback_rpc_urls.len());
        if !self.rpc_url.trim().is_empty() {
            urls.push(self.rpc_url.trim().to_owned());
        }
        for u in &self.fallback_rpc_urls {
            let t = u.trim();
            if t.is_empty() {
                continue;
            }
            if urls.iter().any(|x| x == t) {
                continue;
            }
            urls.push(t.to_owned());
        }
        urls
    }

    fn rpc_for_url(url: &str) -> RpcClient {
        RpcClient::new_with_timeout_and_commitment(
            url.to_owned(),
            Duration::from_secs(20),
            CommitmentConfig::confirmed(),
        )
    }

    async fn with_fallback_and_backoff_cfg<T, Fut>(
        &self,
        cfg: &BackoffConfig,
        context_label: &'static str,
        f: impl Fn(RpcClient) -> Fut + Sync,
    ) -> eyre::Result<T>
    where
        T: Send,
        Fut: std::future::Future<Output = eyre::Result<T>> + Send,
    {
        let urls = self.all_rpc_urls();
        try_all_with_backoff(
            &urls,
            cfg,
            |u| {
                let u = u.clone();
                let f = &f;
                async move {
                    let rpc = Self::rpc_for_url(&u);
                    f(rpc).await
                }
            },
            context_label,
        )
        .await
    }

    async fn with_fallback_and_backoff<T, Fut>(
        &self,
        context_label: &'static str,
        f: impl Fn(RpcClient) -> Fut + Sync,
    ) -> eyre::Result<T>
    where
        T: Send,
        Fut: std::future::Future<Output = eyre::Result<T>> + Send,
    {
        let cfg = BackoffConfig::default();
        self.with_fallback_and_backoff_cfg(&cfg, context_label, f)
            .await
    }

    pub async fn get_genesis_hash(&self) -> eyre::Result<Hash> {
        self.with_fallback_and_backoff("get genesis hash", |rpc| async move {
            let gh = rpc.get_genesis_hash().await.context("get genesis hash")?;
            Ok(gh)
        })
        .await
    }

    pub async fn get_account(&self, key: &Pubkey) -> eyre::Result<Account> {
        let k = *key;
        self.with_fallback_and_backoff("get account", |rpc| async move {
            let a = rpc.get_account(&k).await.context("get account")?;
            Ok(a)
        })
        .await
    }

    pub async fn get_account_optional(&self, key: &Pubkey) -> eyre::Result<Option<Account>> {
        let k = *key;
        self.with_fallback_and_backoff("get account (optional)", |rpc| async move {
            // Prefer an RPC that returns Option rather than treating not-found as an error.
            let resp = rpc
                .get_account_with_commitment(&k, CommitmentConfig::processed())
                .await
                .context("get account")?;
            Ok(resp.value)
        })
        .await
    }

    pub async fn get_latest_blockhash(&self) -> eyre::Result<Hash> {
        self.with_fallback_and_backoff("latest blockhash", |rpc| async move {
            let bh = rpc
                .get_latest_blockhash()
                .await
                .context("latest blockhash")?;
            Ok(bh)
        })
        .await
    }

    pub async fn sign_and_send_instructions(
        &self,
        keypair: &Keypair,
        instructions: Vec<solana_sdk::instruction::Instruction>,
    ) -> eyre::Result<Signature> {
        let instructions = self.with_compute_budget_defaults(instructions);
        let bh = self.get_latest_blockhash().await?;
        let msg = solana_sdk::message::Message::new(&instructions, Some(&keypair.pubkey()));
        let tx = solana_sdk::transaction::Transaction::new(&[keypair], msg, bh);
        let sig = *tx
            .signatures
            .first()
            .ok_or_else(|| eyre::eyre!("missing transaction signature"))?;

        self.with_fallback_and_backoff("simulate tx", |rpc| {
            let tx = tx.clone();
            async move {
                let sim: RpcSimulateTransactionResult = rpc
                    .simulate_transaction(&tx)
                    .await
                    .context("simulate tx")?
                    .value;
                if sim.err.is_some() {
                    eyre::bail!("transaction simulation failed");
                }
                Ok(())
            }
        })
        .await?;

        self.with_fallback_and_backoff("send tx", |rpc| {
            let tx = tx.clone();
            async move {
                rpc.send_transaction(&tx).await.context("send tx")?;
                Ok(())
            }
        })
        .await?;

        let confirm_cfg = BackoffConfig {
            rounds: 12,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(3),
            jitter_max_ms: 200,
        };
        self.with_fallback_and_backoff_cfg(&confirm_cfg, "confirm tx", |rpc| async move {
            let ok = rpc.confirm_transaction(&sig).await.context("confirm tx")?;
            if ok {
                Ok(())
            } else {
                eyre::bail!("transaction not yet confirmed")
            }
        })
        .await?;

        Ok(sig)
    }

    pub async fn sign_and_send_instructions_multi(
        &self,
        fee_payer: &Keypair,
        additional_signers: &[&Keypair],
        instructions: Vec<solana_sdk::instruction::Instruction>,
    ) -> eyre::Result<Signature> {
        let instructions = self.with_compute_budget_defaults(instructions);
        let bh = self.get_latest_blockhash().await?;
        let mut signers: Vec<&Keypair> = Vec::with_capacity(1 + additional_signers.len());
        signers.push(fee_payer);
        signers.extend_from_slice(additional_signers);

        let msg = solana_sdk::message::Message::new(&instructions, Some(&fee_payer.pubkey()));
        let tx = solana_sdk::transaction::Transaction::new(&signers, msg, bh);
        let sig = *tx
            .signatures
            .first()
            .ok_or_else(|| eyre::eyre!("missing transaction signature"))?;

        self.with_fallback_and_backoff("simulate tx", |rpc| {
            let tx = tx.clone();
            async move {
                let sim: RpcSimulateTransactionResult = rpc
                    .simulate_transaction(&tx)
                    .await
                    .context("simulate tx")?
                    .value;
                if sim.err.is_some() {
                    eyre::bail!("transaction simulation failed");
                }
                Ok(())
            }
        })
        .await?;

        self.with_fallback_and_backoff("send tx", |rpc| {
            let tx = tx.clone();
            async move {
                rpc.send_transaction(&tx).await.context("send tx")?;
                Ok(())
            }
        })
        .await?;

        let confirm_cfg = BackoffConfig {
            rounds: 12,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(3),
            jitter_max_ms: 200,
        };
        self.with_fallback_and_backoff_cfg(&confirm_cfg, "confirm tx", |rpc| async move {
            let ok = rpc.confirm_transaction(&sig).await.context("confirm tx")?;
            if ok {
                Ok(())
            } else {
                eyre::bail!("transaction not yet confirmed")
            }
        })
        .await?;

        Ok(sig)
    }

    pub async fn get_program_accounts_bytes(
        &self,
        program_id: Pubkey,
        filters: Vec<RpcFilterType>,
    ) -> eyre::Result<Vec<(Pubkey, Vec<u8>)>> {
        let cfg = RpcProgramAccountsConfig {
            filters: Some(filters),
            account_config: RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                ..Default::default()
            },
            with_context: None,
            sort_results: None,
        };

        let accts = self
            .with_fallback_and_backoff("get program accounts", |rpc| {
                let cfg = cfg.clone();
                async move {
                    rpc.get_program_ui_accounts_with_config(&program_id, cfg)
                        .await
                        .context("get program accounts")
                }
            })
            .await?;

        let mut out = Vec::with_capacity(accts.len());
        for (pk, acc) in accts {
            let Some(data) = acc.data.decode() else {
                continue;
            };
            out.push((pk, data));
        }
        Ok(out)
    }

    pub async fn get_fee_for_message_legacy(
        &self,
        msg: &solana_sdk::message::Message,
    ) -> eyre::Result<u64> {
        let msg = msg.clone();
        self.with_fallback_and_backoff("get fee for message", |rpc| {
            let msg = msg.clone();
            async move {
                let fee = rpc.get_fee_for_message(&msg).await.context("get fee")?;
                Ok(fee)
            }
        })
        .await
    }

    pub async fn get_fee_for_message_v0(
        &self,
        msg: &solana_sdk::message::v0::Message,
    ) -> eyre::Result<u64> {
        let msg = msg.clone();
        self.with_fallback_and_backoff("get fee for message", |rpc| {
            let msg = msg.clone();
            async move {
                let fee = rpc.get_fee_for_message(&msg).await.context("get fee")?;
                Ok(fee)
            }
        })
        .await
    }

    pub fn parse_pubkey(s: &str) -> eyre::Result<Pubkey> {
        Pubkey::from_str(s).context("parse solana pubkey")
    }

    pub async fn get_sol_balance(&self, owner: Pubkey) -> eyre::Result<u64> {
        self.with_fallback_and_backoff("get balance", |rpc| async move {
            let v = rpc.get_balance(&owner).await.context("get balance")?;
            Ok(v)
        })
        .await
    }

    pub async fn get_spl_balance(&self, owner: Pubkey, mint: Pubkey) -> eyre::Result<(u64, u8)> {
        self.with_fallback_and_backoff("get spl balance", |rpc| async move {
            let ata = get_associated_token_address(&owner, &mint);
            let acc = rpc.get_account(&ata).await.context("get token account")?;
            let token =
                spl_token::state::Account::unpack(&acc.data).context("unpack token account")?;
            let mint_acc = rpc.get_account(&mint).await.context("get mint account")?;
            let m = Mint::unpack(&mint_acc.data).context("unpack mint")?;
            Ok((token.amount, m.decimals))
        })
        .await
    }

    pub async fn get_mint_decimals(&self, mint: Pubkey) -> eyre::Result<u8> {
        self.with_fallback_and_backoff("get mint decimals", |rpc| async move {
            let mint_acc = rpc.get_account(&mint).await.context("get mint account")?;
            let m = Mint::unpack(&mint_acc.data).context("unpack mint")?;
            Ok(m.decimals)
        })
        .await
    }

    pub async fn list_nft_like_mints(
        &self,
        owner: Pubkey,
        limit: usize,
    ) -> eyre::Result<Vec<Pubkey>> {
        // SPL token account layout:
        // - mint:   0..32
        // - owner: 32..64
        // We filter server-side to avoid scanning the whole token program.
        let filters = vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
            32,
            owner.to_bytes().to_vec(),
        ))];
        let cfg = RpcProgramAccountsConfig {
            filters: Some(filters),
            account_config: RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                ..Default::default()
            },
            with_context: None,
            sort_results: None,
        };

        let mut out = vec![];
        let accts = self
            .with_fallback_and_backoff("list token accounts", |rpc| {
                let cfg = cfg.clone();
                async move {
                    rpc.get_program_ui_accounts_with_config(&spl_token::id(), cfg)
                        .await
                        .context("get program accounts")
                }
            })
            .await?;

        for (_pk, acc) in accts {
            let Some(data) = acc.data.decode() else {
                continue;
            };
            let Ok(tok) = SplAccount::unpack(&data) else {
                continue;
            };
            if tok.amount != 1 {
                continue;
            }
            // Best-effort: require decimals==0 and supply==1 (typical NFT mint).
            let Ok(mint_acc) = self
                .with_fallback_and_backoff("get mint account", |rpc| {
                    let m = tok.mint;
                    async move { rpc.get_account(&m).await.context("get mint account") }
                })
                .await
            else {
                continue;
            };
            let Ok(mint) = Mint::unpack(&mint_acc.data) else {
                continue;
            };
            if mint.decimals != 0 || mint.supply != 1 {
                continue;
            }
            out.push(tok.mint);
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    pub async fn send_sol(
        &self,
        keypair: &Keypair,
        to: Pubkey,
        lamports: u64,
    ) -> eyre::Result<Signature> {
        let from_addr = Address::new_from_array(keypair.pubkey().to_bytes());
        let to_addr = Address::new_from_array(to.to_bytes());
        let ix = system_instruction::transfer(&from_addr, &to_addr, lamports);

        let bh = self
            .with_fallback_and_backoff("latest blockhash", |rpc| async move {
                let bh = rpc
                    .get_latest_blockhash()
                    .await
                    .context("latest blockhash")?;
                Ok(bh)
            })
            .await?;

        let msg = solana_sdk::message::Message::new(&[ix], Some(&keypair.pubkey()));
        let tx = solana_sdk::transaction::Transaction::new(&[keypair], msg, bh);
        let sig = *tx
            .signatures
            .first()
            .ok_or_else(|| eyre::eyre!("missing transaction signature"))?;

        // Simulate before broadcast to catch obvious failures (rent/insufficient funds,
        // program errors) before we submit.
        self.with_fallback_and_backoff("simulate sol tx", |rpc| {
            let tx = tx.clone();
            async move {
                let sim: RpcSimulateTransactionResult = rpc
                    .simulate_transaction(&tx)
                    .await
                    .context("simulate tx")?
                    .value;
                if sim.err.is_some() {
                    eyre::bail!("transaction simulation failed");
                }
                Ok(())
            }
        })
        .await?;

        // Broadcast across multiple RPCs; confirm best-effort with backoff.
        self.with_fallback_and_backoff("send sol tx", |rpc| {
            let tx = tx.clone();
            async move {
                rpc.send_transaction(&tx).await.context("send tx")?;
                Ok(())
            }
        })
        .await?;

        let confirm_cfg = BackoffConfig {
            rounds: 10,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(3),
            jitter_max_ms: 200,
        };
        self.with_fallback_and_backoff_cfg(&confirm_cfg, "confirm sol tx", |rpc| async move {
            let ok = rpc.confirm_transaction(&sig).await.context("confirm tx")?;
            if ok {
                Ok(())
            } else {
                eyre::bail!("transaction not yet confirmed")
            }
        })
        .await?;

        Ok(sig)
    }

    pub async fn request_airdrop(&self, to: Pubkey, lamports: u64) -> eyre::Result<Signature> {
        let sig = self
            .with_fallback_and_backoff("request airdrop", |rpc| async move {
                let sig = rpc
                    .request_airdrop(&to, lamports)
                    .await
                    .context("request airdrop")?;
                Ok(sig)
            })
            .await?;

        // Best-effort confirm; on some clusters it may take a moment.
        let confirm_cfg = BackoffConfig {
            rounds: 12,
            base_delay: Duration::from_millis(300),
            max_delay: Duration::from_secs(4),
            jitter_max_ms: 250,
        };
        self.with_fallback_and_backoff_cfg(&confirm_cfg, "confirm airdrop", |rpc| async move {
            let ok = rpc
                .confirm_transaction(&sig)
                .await
                .context("confirm airdrop")?;
            if ok {
                Ok(())
            } else {
                eyre::bail!("airdrop not yet confirmed")
            }
        })
        .await?;
        Ok(sig)
    }

    pub async fn send_spl(
        &self,
        keypair: &Keypair,
        to_owner: Pubkey,
        mint: Pubkey,
        amount: u64,
    ) -> eyre::Result<Signature> {
        let from_owner = keypair.pubkey();
        let from_ata = get_associated_token_address(&from_owner, &mint);
        let to_ata = get_associated_token_address(&to_owner, &mint);

        let mut ixs = vec![];

        // Create recipient ATA if missing.
        let to_ata_exists = self
            .with_fallback_and_backoff("check recipient ata", |rpc| async move {
                Ok(rpc.get_account(&to_ata).await.is_ok())
            })
            .await
            .unwrap_or(false);
        if !to_ata_exists {
            ixs.push(
                spl_associated_token_account::instruction::create_associated_token_account(
                    &from_owner,
                    &to_owner,
                    &mint,
                    &spl_token::id(),
                ),
            );
        }

        // Mint decimals for checked transfer.
        let m = self
            .with_fallback_and_backoff("get mint account", |rpc| async move {
                let mint_acc = rpc.get_account(&mint).await.context("get mint account")?;
                let m = Mint::unpack(&mint_acc.data).context("unpack mint")?;
                Ok(m)
            })
            .await?;

        ixs.push(
            spl_token::instruction::transfer_checked(
                &spl_token::id(),
                &from_ata,
                &mint,
                &to_ata,
                &from_owner,
                &[],
                amount,
                m.decimals,
            )
            .context("build spl transfer")?,
        );

        let bh = self
            .with_fallback_and_backoff("latest blockhash", |rpc| async move {
                let bh = rpc
                    .get_latest_blockhash()
                    .await
                    .context("latest blockhash")?;
                Ok(bh)
            })
            .await?;
        let msg = solana_sdk::message::Message::new(&ixs, Some(&from_owner));
        let tx = solana_sdk::transaction::Transaction::new(&[keypair], msg, bh);
        let sig = *tx
            .signatures
            .first()
            .ok_or_else(|| eyre::eyre!("missing transaction signature"))?;

        self.with_fallback_and_backoff("simulate spl tx", |rpc| {
            let tx = tx.clone();
            async move {
                let sim: RpcSimulateTransactionResult = rpc
                    .simulate_transaction(&tx)
                    .await
                    .context("simulate tx")?
                    .value;
                if sim.err.is_some() {
                    eyre::bail!("transaction simulation failed");
                }
                Ok(())
            }
        })
        .await?;

        self.with_fallback_and_backoff("send spl tx", |rpc| {
            let tx = tx.clone();
            async move {
                rpc.send_transaction(&tx).await.context("send tx")?;
                Ok(())
            }
        })
        .await?;
        let confirm_cfg = BackoffConfig {
            rounds: 10,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(3),
            jitter_max_ms: 200,
        };
        self.with_fallback_and_backoff_cfg(&confirm_cfg, "confirm spl tx", |rpc| async move {
            let ok = rpc.confirm_transaction(&sig).await.context("confirm tx")?;
            if ok {
                Ok(())
            } else {
                eyre::bail!("transaction not yet confirmed")
            }
        })
        .await?;
        Ok(sig)
    }

    pub async fn jupiter_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u32,
    ) -> eyre::Result<Value> {
        let base = self.jupiter_base_url.trim();
        if !base.starts_with("https://") && !is_loopback_http(base) && !allow_insecure_http() {
            eyre::bail!(
                "jupiter_base_url must use https (or loopback); set SEASHAIL_ALLOW_INSECURE_HTTP=1 to override"
            );
        }
        let url = format!(
            "{}/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}&swapMode=ExactIn",
            self.jupiter_base_url, input_mint, output_mint, amount, slippage_bps
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .context("build http client")?;
        let mut req = client.get(url);
        if let Some(k) = self
            .jupiter_api_key
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            req = req.header("x-api-key", k);
        }
        let v: Value = req
            .send()
            .await
            .context("jupiter quote request")?
            .error_for_status()
            .context("jupiter quote status")?
            .json()
            .await
            .context("jupiter quote json")?;
        Ok(v)
    }

    pub async fn jupiter_swap_tx(
        &self,
        quote_response: Value,
        user_pubkey: Pubkey,
    ) -> eyre::Result<Vec<u8>> {
        let base = self.jupiter_base_url.trim();
        if !base.starts_with("https://") && !is_loopback_http(base) && !allow_insecure_http() {
            eyre::bail!(
                "jupiter_base_url must use https (or loopback); set SEASHAIL_ALLOW_INSECURE_HTTP=1 to override"
            );
        }
        let url = format!("{}/swap", self.jupiter_base_url);
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .context("build http client")?;
        let body = serde_json::json!({
          "quoteResponse": quote_response,
          "userPublicKey": user_pubkey.to_string(),
          "wrapAndUnwrapSol": true,
          "dynamicComputeUnitLimit": true
        });
        let mut req = client.post(url);
        if let Some(k) = self
            .jupiter_api_key
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            req = req.header("x-api-key", k);
        }
        let v: Value = req
            .json(&body)
            .send()
            .await
            .context("jupiter swap request")?
            .error_for_status()
            .context("jupiter swap status")?
            .json()
            .await
            .context("jupiter swap json")?;
        let tx_b64 = v
            .get("swapTransaction")
            .and_then(|x| x.as_str())
            .ok_or_else(|| eyre::eyre!("missing swapTransaction"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(tx_b64)
            .context("decode swapTransaction")?;
        Ok(bytes)
    }

    fn validate_remote_versioned_message(
        user: Pubkey,
        msg: &VersionedMessage,
        skip_provider_check: bool,
    ) -> eyre::Result<()> {
        // Remote-constructed transactions (e.g. Jupiter) are untrusted input.
        // Enforce a strict minimum: fee payer must be the user, and the only required signer
        // must be the user. Anything else is refused.
        let keys = msg.static_account_keys();
        if keys.is_empty() {
            eyre::bail!("invalid transaction: missing account keys");
        }
        let user_addr = Address::new_from_array(user.to_bytes());
        let fee_payer = keys
            .first()
            .ok_or_else(|| eyre::eyre!("invalid transaction: missing fee payer"))?;
        if *fee_payer != user_addr {
            eyre::bail!("refusing transaction: fee payer is not the user");
        }
        let hdr = msg.header();
        if hdr.num_required_signatures != 1 {
            eyre::bail!("refusing transaction: unexpected number of required signatures");
        }

        // Provider allowlisting: when Seashail uses Jupiter's Swap API, require the
        // transaction to actually invoke a known Jupiter program id. This is not a full
        // instruction-level policy (routes can touch many AMM programs), but it prevents obvious
        // misbinding where the remote "swapTransaction" isn't a Jupiter swap at all.
        //
        // References (program IDs are stable across time):
        // - Jupiter v6: JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4
        // - Jupiter v4: JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB
        // - Jupiter v2: JUP2jxvQffg8W4Jw8dYpQ8eQJ8v1s5p8yL6kD3m1j7d
        if skip_provider_check {
            return Ok(());
        }
        let jupiter_addrs: Vec<Address> = JUPITER_PROGRAMS
            .iter()
            .copied()
            .map(Self::parse_pubkey)
            .collect::<Result<Vec<_>, _>>()
            .context("parse hardcoded jupiter program ids")?
            .into_iter()
            .map(|pk| Address::new_from_array(pk.to_bytes()))
            .collect();

        let pids = program_ids(msg, keys)?;
        let invokes_jupiter = pids
            .iter()
            .any(|pid| jupiter_addrs.iter().any(|j| *pid == j));
        if !invokes_jupiter {
            eyre::bail!("refusing transaction: does not invoke a known Jupiter program id");
        }
        Ok(())
    }

    fn validate_remote_versioned_message_allowlist(
        user: Pubkey,
        msg: &VersionedMessage,
        allowed_program_ids: &[Pubkey],
    ) -> eyre::Result<()> {
        // Remote-constructed transactions are untrusted input.
        // Enforce a strict minimum: fee payer must be the user, and the only required signer
        // must be the user. Anything else is refused.
        let keys = msg.static_account_keys();
        if keys.is_empty() {
            eyre::bail!("invalid transaction: missing account keys");
        }
        let user_addr = Address::new_from_array(user.to_bytes());
        let fee_payer = keys
            .first()
            .ok_or_else(|| eyre::eyre!("invalid transaction: missing fee payer"))?;
        if *fee_payer != user_addr {
            eyre::bail!("refusing transaction: fee payer is not the user");
        }
        let hdr = msg.header();
        if hdr.num_required_signatures != 1 {
            eyre::bail!("refusing transaction: unexpected number of required signatures");
        }

        if allowed_program_ids.is_empty() {
            eyre::bail!("refusing transaction: empty program allowlist");
        }
        let allowed_addrs: Vec<Address> = allowed_program_ids
            .iter()
            .map(|pk| Address::new_from_array(pk.to_bytes()))
            .collect();

        let pids = program_ids(msg, keys)?;
        if pids.is_empty() {
            eyre::bail!("invalid transaction: missing instructions");
        }
        // Require every instruction program id to be allowlisted. This is strict, but it makes
        // the security story understandable: remote tx bytes can only invoke programs you
        // explicitly permit (plus any shared/common programs you choose to include).
        for pid in pids {
            let ok = allowed_addrs.iter().any(|a| pid == a);
            if !ok {
                eyre::bail!("refusing transaction: invokes a non-allowlisted program id");
            }
        }
        Ok(())
    }

    pub async fn sign_and_send_versioned_allowlist(
        &self,
        keypair: &Keypair,
        tx_bytes: &[u8],
        allowed_program_ids: &[Pubkey],
    ) -> eyre::Result<Signature> {
        let vt: VersionedTransaction = bincode::DefaultOptions::new()
            .with_limit(MAX_REMOTE_TX_BYTES)
            .deserialize(tx_bytes)
            .context("deserialize versioned tx")?;
        let msg: VersionedMessage = vt.message;
        Self::validate_remote_versioned_message_allowlist(
            keypair.pubkey(),
            &msg,
            allowed_program_ids,
        )?;
        let signed = VersionedTransaction::try_new(msg, &[keypair]).context("sign tx")?;
        let sig = *signed
            .signatures
            .first()
            .ok_or_else(|| eyre::eyre!("missing transaction signature"))?;

        self.with_fallback_and_backoff("simulate versioned tx", |rpc| {
            let signed = signed.clone();
            async move {
                let sim: RpcSimulateTransactionResult = rpc
                    .simulate_transaction(&signed)
                    .await
                    .context("simulate versioned tx")?
                    .value;
                if sim.err.is_some() {
                    eyre::bail!("transaction simulation failed");
                }
                Ok(())
            }
        })
        .await?;

        self.with_fallback_and_backoff("send versioned tx", |rpc| {
            let signed = signed.clone();
            async move {
                rpc.send_transaction(&signed)
                    .await
                    .context("send versioned tx")?;
                Ok(())
            }
        })
        .await?;

        let confirm_cfg = BackoffConfig {
            rounds: 12,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(3),
            jitter_max_ms: 200,
        };
        self.with_fallback_and_backoff_cfg(
            &confirm_cfg,
            "confirm versioned tx",
            |rpc| async move {
                let ok = rpc
                    .confirm_transaction(&sig)
                    .await
                    .context("confirm versioned tx")?;
                if ok {
                    Ok(())
                } else {
                    eyre::bail!("transaction not yet confirmed")
                }
            },
        )
        .await?;
        Ok(sig)
    }

    pub async fn sign_and_send_versioned(
        &self,
        keypair: &Keypair,
        tx_bytes: &[u8],
    ) -> eyre::Result<Signature> {
        let vt: VersionedTransaction = bincode::DefaultOptions::new()
            .with_limit(MAX_REMOTE_TX_BYTES)
            .deserialize(tx_bytes)
            .context("deserialize versioned tx")?;
        let msg: VersionedMessage = vt.message;
        // Allow local Jupiter mocks for E2E testing/dev (loopback base URL). Real Jupiter hosts
        // must pass provider allowlisting.
        let skip_provider_check = is_loopback_http(self.jupiter_base_url.trim());
        Self::validate_remote_versioned_message(keypair.pubkey(), &msg, skip_provider_check)?;
        let signed = VersionedTransaction::try_new(msg, &[keypair]).context("sign tx")?;
        let sig = *signed
            .signatures
            .first()
            .ok_or_else(|| eyre::eyre!("missing transaction signature"))?;

        self.with_fallback_and_backoff("simulate versioned tx", |rpc| {
            let signed = signed.clone();
            async move {
                let sim: RpcSimulateTransactionResult = rpc
                    .simulate_transaction(&signed)
                    .await
                    .context("simulate versioned tx")?
                    .value;
                if sim.err.is_some() {
                    eyre::bail!("transaction simulation failed");
                }
                Ok(())
            }
        })
        .await?;

        self.with_fallback_and_backoff("send versioned tx", |rpc| {
            let signed = signed.clone();
            async move {
                rpc.send_transaction(&signed)
                    .await
                    .context("send versioned tx")?;
                Ok(())
            }
        })
        .await?;

        let confirm_cfg = BackoffConfig {
            rounds: 12,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(3),
            jitter_max_ms: 200,
        };
        self.with_fallback_and_backoff_cfg(
            &confirm_cfg,
            "confirm versioned tx",
            |rpc| async move {
                let ok = rpc
                    .confirm_transaction(&sig)
                    .await
                    .context("confirm versioned tx")?;
                if ok {
                    Ok(())
                } else {
                    eyre::bail!("transaction not yet confirmed")
                }
            },
        )
        .await?;
        Ok(sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::instruction::Instruction;

    #[test]
    fn prepends_compute_budget_instructions_when_configured() -> eyre::Result<()> {
        let sol = SolanaChain::new_with_fallbacks(
            "http://127.0.0.1:8899",
            &[],
            "http://127.0.0.1:0",
            None,
            Some(1_400_000),
            Some(50_000),
        );
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        let ixs: Vec<Instruction> = vec![system_instruction::transfer(&a, &b, 1)];
        let out = sol.with_compute_budget_defaults(ixs);
        let first = out.first().ok_or_else(|| eyre::eyre!("missing first ix"))?;
        assert_eq!(first.program_id, compute_budget_program_id());
        Ok(())
    }

    #[test]
    fn does_not_duplicate_compute_budget_instructions() {
        let sol = SolanaChain::new_with_fallbacks(
            "http://127.0.0.1:8899",
            &[],
            "http://127.0.0.1:0",
            None,
            Some(1_400_000),
            Some(50_000),
        );
        let mut ixs: Vec<Instruction> = vec![compute_budget_set_compute_unit_limit(10_000)];
        let a = Pubkey::new_unique();
        let b = Pubkey::new_unique();
        ixs.push(system_instruction::transfer(&a, &b, 1));
        let out = sol.with_compute_budget_defaults(ixs);
        // If the caller already provided ComputeBudget, we preserve as-is.
        assert_eq!(out.len(), 2);
    }
}

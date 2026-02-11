use crate::config::HttpConfig;
use crate::retry::{try_all_with_backoff, BackoffConfig};
use alloy::{
    consensus::{SignableTransaction as _, TxEip1559, TxEnvelope, TxLegacy},
    network::TransactionBuilder as _,
    primitives::{keccak256, Address, Bytes, TxKind, B256, U256},
    providers::{Provider as _, RootProvider},
    rpc::types::{BlockNumberOrTag, TransactionReceipt, TransactionRequest},
    signers::{local::PrivateKeySigner, SignerSync as _},
    sol,
    sol_types::SolCall as _,
};
use eyre::Context as _;
use reqwest::Client;
use serde::Deserialize;
use std::{str::FromStr as _, time::Duration};
use tokio::time::sleep;

const ONEINCH_ROUTER: &str = "0x1111111254eeb25477b68fb85ed929f73a960582";
const ONEINCH_NATIVE_SENTINEL: &str = "0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE";
const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

type EvmProvider = RootProvider;

/// Extract the lower 128 bits from a U256 (equivalent to ethers' `low_u128()`).
pub fn u256_low_u128(v: U256) -> u128 {
    let limbs = v.as_limbs();
    u128::from(limbs[0]) | (u128::from(limbs[1]) << 64)
}

pub fn compute_eip1559_fees(base_fee: u128, gas_price: u128) -> (u128, u128) {
    // Conservative fee policy:
    // - priority: max(1.5 gwei, gas_price / 10)
    // - max_fee: base_fee * 2 + priority
    let min_priority: u128 = 1_500_000_000; // 1.5 gwei
    let priority = std::cmp::max(min_priority, gas_price / 10);

    let mut max_fee = base_fee.saturating_mul(2).saturating_add(priority);
    let min_fee = base_fee.saturating_add(priority);
    if max_fee < min_fee {
        max_fee = min_fee;
    }
    (max_fee, priority)
}

/// Apply Seashail's "prefer EIP-1559 when supported" fee policy to a transaction.
///
/// This is a pure helper so we can unit-test fee selection without RPC/provider variance.
pub fn apply_fee_policy(
    mut tx: TransactionRequest,
    base_fee: Option<u128>,
    gas_price: u128,
    from: Address,
    chain_id: u64,
) -> TransactionRequest {
    // If caller already set explicit fee fields, don't override them.
    if tx.max_fee_per_gas.is_some()
        || tx.max_priority_fee_per_gas.is_some()
        || tx.gas_price.is_some()
    {
        return tx;
    }

    if tx.chain_id.is_none() {
        tx.chain_id = Some(chain_id);
    }
    if tx.from.is_none() {
        tx.from = Some(from);
    }

    if let Some(base_fee) = base_fee {
        let (max_fee, priority) = compute_eip1559_fees(base_fee, gas_price);
        tx.max_fee_per_gas = Some(max_fee);
        tx.max_priority_fee_per_gas = Some(priority);
    } else {
        tx.gas_price = Some(gas_price);
    }
    tx
}

fn broadcast_err_is_ok(err: &eyre::Report) -> bool {
    let s = format!("{err:#}").to_lowercase();
    s.contains("already known")
        || s.contains("known transaction")
        || s.contains("already imported")
        || s.contains("already in mempool")
}

/// Build and sign a consensus transaction from a fully-populated `TransactionRequest`.
fn build_and_sign_tx(
    signer: &PrivateKeySigner,
    tx: &TransactionRequest,
) -> eyre::Result<(TxEnvelope, B256)> {
    let to = tx.to.unwrap_or(TxKind::Create);
    let value = tx.value.unwrap_or(U256::ZERO);
    let input = tx.input.clone().into_input().unwrap_or_default();
    let nonce = tx.nonce.unwrap_or(0);
    let gas_limit = tx.gas.unwrap_or(21_000);

    if tx.max_fee_per_gas.is_some() {
        // EIP-1559
        let consensus_tx = TxEip1559 {
            chain_id: tx.chain_id.unwrap_or(1),
            nonce,
            gas_limit,
            max_fee_per_gas: tx.max_fee_per_gas.unwrap_or(0),
            max_priority_fee_per_gas: tx.max_priority_fee_per_gas.unwrap_or(0),
            to,
            value,
            input,
            access_list: tx.access_list.clone().unwrap_or_default(),
        };
        let hash = consensus_tx.signature_hash();
        let sig = signer.sign_hash_sync(&hash).context("sign eip1559")?;
        let signed_tx = consensus_tx.into_signed(sig);
        let tx_hash = *signed_tx.hash();
        Ok((TxEnvelope::Eip1559(signed_tx), tx_hash))
    } else {
        // Legacy
        let consensus_tx = TxLegacy {
            chain_id: tx.chain_id,
            nonce,
            gas_price: tx.gas_price.unwrap_or(0),
            gas_limit,
            to,
            value,
            input,
        };
        let hash = consensus_tx.signature_hash();
        let sig = signer.sign_hash_sync(&hash).context("sign legacy")?;
        let signed_tx = consensus_tx.into_signed(sig);
        let tx_hash = *signed_tx.hash();
        Ok((TxEnvelope::Legacy(signed_tx), tx_hash))
    }
}

sol! {
    #[sol(rpc)]
    contract IERC20 {
        function name() external view returns (string);
        function balanceOf(address account) external view returns (uint256);
        function totalSupply() external view returns (uint256);
        function decimals() external view returns (uint8);
        function symbol() external view returns (string);
        function allowance(address owner, address spender) external view returns (uint256);
        function transfer(address to, uint256 value) returns (bool);
        function approve(address spender, uint256 value) returns (bool);
    }
}

sol! {
    #[sol(rpc)]
    contract IQuoterV2 {
        struct QuoteExactInputSingleParams {
            address tokenIn;
            address tokenOut;
            uint256 amountIn;
            uint24 fee;
            uint160 sqrtPriceLimitX96;
        }
        function quoteExactInputSingle(QuoteExactInputSingleParams params)
            external returns (uint256 amountOut, uint160 sqrtPriceX96After, uint32 initializedTicksCrossed, uint256 gasEstimate);
    }
}

sol! {
    #[sol(rpc)]
    contract ISwapRouter02 {
        struct ExactInputSingleParams {
            address tokenIn;
            address tokenOut;
            uint24 fee;
            address recipient;
            uint256 amountIn;
            uint256 amountOutMinimum;
            uint160 sqrtPriceLimitX96;
        }
        function exactInputSingle(ExactInputSingleParams params)
            external payable returns (uint256 amountOut);
        function multicall(bytes[] data) external payable returns (bytes[] results);
        function unwrapWETH9(uint256 amountMinimum, address recipient) external payable;
        function refundETH() external payable;
    }
}

sol! {
    function safeTransferFrom(address from, address to, uint256 tokenId);
}

#[derive(Debug, Clone)]
pub struct UniswapAddresses {
    pub router02: Address,
    pub quoter_v2: Address,
    pub wrapped_native: Address,
    pub usdc: Address,
}

#[derive(Debug, Clone)]
pub struct UniswapSwapRequest {
    pub from: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: U256,
    pub amount_out_min: U256,
    pub fee: u32,
    pub native_in: bool,
    pub native_out: bool,
}

#[derive(Debug, Clone)]
pub struct EvmChain {
    pub name: String,
    pub chain_id: u64,
    pub rpc_url: String,
    pub fallback_rpc_urls: Vec<String>,
    pub uniswap: Option<UniswapAddresses>,
    pub oneinch: OneInchConfig,
}

#[derive(Debug, Clone)]
pub struct OneInchConfig {
    pub base_url: String,
    pub api_key: Option<String>,
}

impl EvmChain {
    pub fn for_name(name: &str, chain_id: u64, rpc_url: &str, http: &HttpConfig) -> Self {
        let (fallback_rpc_urls, uniswap) = defaults_for(name);
        Self {
            name: name.to_owned(),
            chain_id,
            rpc_url: rpc_url.to_owned(),
            fallback_rpc_urls,
            uniswap,
            oneinch: OneInchConfig {
                base_url: http.oneinch_base_url.clone(),
                api_key: http.oneinch_api_key.clone(),
            },
        }
    }

    fn provider_for_url(url: &str) -> eyre::Result<EvmProvider> {
        let u: reqwest::Url = url
            .parse()
            .with_context(|| format!("invalid rpc url: {url}"))?;
        let client = Client::builder()
            .timeout(DEFAULT_RPC_TIMEOUT)
            .connect_timeout(DEFAULT_RPC_CONNECT_TIMEOUT)
            .build()
            .context("build rpc http client")?;
        let http = alloy::transports::http::Http::with_client(client, u);
        let rpc_client = alloy::rpc::client::RpcClient::new(http, false);
        Ok(RootProvider::new(rpc_client))
    }

    pub fn provider(&self) -> eyre::Result<EvmProvider> {
        Self::provider_for_url(self.rpc_url.as_str())
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

    async fn with_fallback_and_backoff<T, Fut>(
        &self,
        context_label: &'static str,
        f: impl Fn(EvmProvider) -> Fut + Sync,
    ) -> eyre::Result<T>
    where
        T: Send,
        Fut: std::future::Future<Output = eyre::Result<T>> + Send,
    {
        let urls = self.all_rpc_urls();
        let cfg = BackoffConfig::default();
        try_all_with_backoff(
            &urls,
            &cfg,
            |u| {
                let u = u.clone();
                let f = &f;
                async move {
                    let p = Self::provider_for_url(&u)?;
                    f(p).await
                }
            },
            context_label,
        )
        .await
    }

    pub async fn get_native_balance(&self, addr: Address) -> eyre::Result<U256> {
        self.with_fallback_and_backoff("get balance", |p| async move {
            let v = p.get_balance(addr).await.context("get balance")?;
            Ok(v)
        })
        .await
    }

    pub async fn get_erc20_balance(
        &self,
        token: Address,
        owner: Address,
    ) -> eyre::Result<(U256, u8, String)> {
        self.with_fallback_and_backoff("erc20 balance", |p| async move {
            let c = IERC20::new(token, &p);
            let bal = c.balanceOf(owner).call().await.context("erc20 balanceOf")?;
            let decimals = c.decimals().call().await.context("erc20 decimals")?;
            let symbol = c
                .symbol()
                .call()
                .await
                .unwrap_or_else(|_| "ERC20".to_owned());
            Ok((bal, decimals, symbol))
        })
        .await
    }

    pub async fn get_erc20_metadata(&self, token: Address) -> eyre::Result<(u8, String)> {
        self.with_fallback_and_backoff("erc20 metadata", |p| async move {
            let c = IERC20::new(token, &p);
            let decimals = c.decimals().call().await.context("erc20 decimals")?;
            let symbol = c
                .symbol()
                .call()
                .await
                .unwrap_or_else(|_| "ERC20".to_owned());
            Ok((decimals, symbol))
        })
        .await
    }

    pub async fn get_erc20_details(
        &self,
        token: Address,
    ) -> eyre::Result<(u8, String, String, U256)> {
        self.with_fallback_and_backoff("erc20 details", |p| async move {
            let c = IERC20::new(token, &p);
            let decimals = c.decimals().call().await.context("erc20 decimals")?;
            let symbol = c
                .symbol()
                .call()
                .await
                .unwrap_or_else(|_| "ERC20".to_owned());
            let name = c.name().call().await.unwrap_or_else(|_| "ERC20".to_owned());
            let supply = c.totalSupply().call().await.context("erc20 totalSupply")?;
            Ok((decimals, symbol, name, supply))
        })
        .await
    }

    pub async fn get_contract_code(&self, addr: Address) -> eyre::Result<Bytes> {
        self.with_fallback_and_backoff("get code", |p| async move {
            let code = p.get_code_at(addr).await.context("get code")?;
            Ok(code)
        })
        .await
    }

    pub async fn get_storage_at(&self, addr: Address, slot: B256) -> eyre::Result<B256> {
        self.with_fallback_and_backoff("get storage", |p| async move {
            let slot_u256 = U256::from_be_bytes(slot.0);
            let v = p
                .get_storage_at(addr, slot_u256)
                .await
                .context("get storage")?;
            Ok(B256::from(v))
        })
        .await
    }

    pub fn eip1967_implementation_slot() -> B256 {
        // bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)
        let h = keccak256("eip1967.proxy.implementation");
        let v = U256::from_be_bytes(h.0) - U256::from(1_u64);
        B256::from(v)
    }

    pub async fn estimate_tx_gas(&self, tx: &TransactionRequest) -> eyre::Result<u64> {
        let txc = tx.clone();
        self.with_fallback_and_backoff("estimate gas", |p| {
            let tx_inner = txc.clone();
            async move {
                let gas = p.estimate_gas(tx_inner).await.context("estimate gas")?;
                Ok(gas)
            }
        })
        .await
    }

    /// Estimate gas using the configured primary RPC only.
    ///
    /// Rationale (security): for write-path "fail closed" simulation, using fallback RPCs can
    /// mask reverts on the primary network (e.g., local Anvil) and lead to confusing or unsafe
    /// behavior. Use this for pre-signing checks.
    pub async fn estimate_tx_gas_strict(&self, tx: &TransactionRequest) -> eyre::Result<u64> {
        let p = self.provider()?;
        let gas = p.estimate_gas(tx.clone()).await.context("estimate gas")?;
        Ok(gas)
    }

    /// Simulate using the configured primary RPC only. See `estimate_tx_gas_strict`.
    pub async fn simulate_tx_strict(&self, tx: &TransactionRequest) -> eyre::Result<Bytes> {
        let p = self.provider()?;
        let out = p
            .call(tx.clone())
            .block(BlockNumberOrTag::Pending.into())
            .await
            .context("eth_call")?;
        Ok(out)
    }

    async fn pick_healthy_provider(&self) -> eyre::Result<EvmProvider> {
        let urls = self.all_rpc_urls();
        let cfg = BackoffConfig::default();
        try_all_with_backoff(
            &urls,
            &cfg,
            |u| {
                let u = u.clone();
                async move {
                    let p = Self::provider_for_url(&u)?;
                    // Basic liveness check.
                    p.get_block_number().await.context("get block number")?;
                    Ok(p)
                }
            },
            "select rpc",
        )
        .await
    }

    pub async fn send_tx(
        &self,
        signer: PrivateKeySigner,
        mut tx: TransactionRequest,
    ) -> eyre::Result<B256> {
        let provider = self.pick_healthy_provider().await?;
        let from = signer.address();

        tx.chain_id = Some(self.chain_id);
        if tx.from.is_none() {
            tx.from = Some(from);
        }

        // Prefer EIP-1559 fees when the chain supports base fees.
        if tx.gas_price.is_none() && tx.max_fee_per_gas.is_none() {
            let base_fee = provider
                .get_block_by_number(BlockNumberOrTag::Pending)
                .await
                .ok()
                .flatten()
                .and_then(|b| b.header.base_fee_per_gas.map(u128::from));

            let gp = provider.get_gas_price().await.context("get gas price")?;
            tx = apply_fee_policy(tx, base_fee, gp, from, self.chain_id);
        }

        if tx.nonce.is_none() {
            let n = provider
                .get_transaction_count(from)
                .pending()
                .await
                .context("get nonce")?;
            tx.nonce = Some(n);
        }

        if tx.gas.is_none() {
            let gas = provider
                .estimate_gas(tx.clone())
                .await
                .context("estimate gas")?;
            // Add a small buffer for flaky estimators.
            let gas = gas.saturating_mul(120) / 100;
            tx.gas = Some(gas);
        }

        // Sign once; then broadcast the same raw tx across multiple RPCs.
        let (envelope, tx_hash) = build_and_sign_tx(&signer, &tx).context("sign tx")?;
        let raw_bytes = alloy::eips::eip2718::Encodable2718::encoded_2718(&envelope);

        let urls = self.all_rpc_urls();
        let cfg = BackoffConfig::default();
        try_all_with_backoff(
            &urls,
            &cfg,
            |u| {
                let u = u.clone();
                let raw_bytes = raw_bytes.clone();
                async move {
                    let p = Self::provider_for_url(&u)?;
                    match p.send_raw_transaction(&raw_bytes).await {
                        Ok(_pending) => Ok(()),
                        Err(e) => {
                            let ae: eyre::Report = e.into();
                            if broadcast_err_is_ok(&ae) {
                                Ok(())
                            } else {
                                Err(ae).context("broadcast raw tx")
                            }
                        }
                    }
                }
            },
            "send transaction",
        )
        .await?;

        Ok(tx_hash)
    }

    pub async fn get_tx_receipt(&self, tx: B256) -> eyre::Result<Option<TransactionReceipt>> {
        self.with_fallback_and_backoff("get tx receipt", |p| async move {
            let r = p
                .get_transaction_receipt(tx)
                .await
                .context("get transaction receipt")?;
            Ok(r)
        })
        .await
    }

    pub async fn wait_for_tx_receipt(
        &self,
        tx: B256,
        timeout: Duration,
    ) -> eyre::Result<TransactionReceipt> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                eyre::bail!("timed out waiting for tx receipt");
            }
            if let Some(r) = self.get_tx_receipt(tx).await? {
                return Ok(r);
            }
            sleep(Duration::from_millis(250)).await;
        }
    }

    pub fn build_native_transfer(from: Address, to: Address, value: U256) -> TransactionRequest {
        TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_value(value)
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn build_erc20_transfer(
        &self,
        from: Address,
        token: Address,
        to: Address,
        value: U256,
    ) -> eyre::Result<TransactionRequest> {
        let calldata = IERC20::transferCall { to, value }.abi_encode();
        Ok(TransactionRequest::default()
            .with_from(from)
            .with_to(token)
            .with_input(Bytes::from(calldata)))
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn build_erc20_approve(
        &self,
        from: Address,
        token: Address,
        spender: Address,
        value: U256,
    ) -> eyre::Result<TransactionRequest> {
        let calldata = IERC20::approveCall { spender, value }.abi_encode();
        Ok(TransactionRequest::default()
            .with_from(from)
            .with_to(token)
            .with_input(Bytes::from(calldata)))
    }

    pub fn build_erc721_safe_transfer_from(
        from: Address,
        contract: Address,
        to: Address,
        token_id: U256,
    ) -> TransactionRequest {
        let calldata = safeTransferFromCall {
            from,
            to,
            tokenId: token_id,
        }
        .abi_encode();
        TransactionRequest::default()
            .with_from(from)
            .with_to(contract)
            .with_input(Bytes::from(calldata))
    }

    pub async fn erc20_allowance(
        &self,
        token: Address,
        owner: Address,
        spender: Address,
    ) -> eyre::Result<U256> {
        self.with_fallback_and_backoff("erc20 allowance", |p| async move {
            let c = IERC20::new(token, &p);
            let v = c
                .allowance(owner, spender)
                .call()
                .await
                .context("erc20 allowance")?;
            Ok(v)
        })
        .await
    }

    pub fn parse_address(s: &str) -> eyre::Result<Address> {
        Address::from_str(s).context("parse evm address")
    }

    fn oneinch_auth_header(&self) -> eyre::Result<String> {
        let Some(k) = &self.oneinch.api_key else {
            eyre::bail!("1inch requires an API key (set http.oneinch_api_key in config.toml)");
        };
        Ok(format!("Bearer {}", k.trim()))
    }

    fn oneinch_base_url_is_allowed(&self) -> bool {
        fn host_prefix_ok(s: &str, prefix: &str) -> bool {
            if !s.starts_with(prefix) {
                return false;
            }
            matches!(s.as_bytes().get(prefix.len()), None | Some(b':' | b'/'))
        }

        let s = self.oneinch.base_url.trim();
        if s.starts_with("https://") {
            return true;
        }
        if !s.starts_with("http://") {
            return false;
        }
        host_prefix_ok(s, "http://127.0.0.1")
            || host_prefix_ok(s, "http://localhost")
            || host_prefix_ok(s, "http://[::1]")
    }

    pub async fn oneinch_spender(&self) -> eyre::Result<Address> {
        #[derive(Debug, Deserialize)]
        struct Resp {
            address: String,
        }

        if !self.oneinch_base_url_is_allowed() {
            eyre::bail!("oneinch_base_url must use https (or http://localhost for local testing)");
        }
        let url = format!(
            "{}/{}/approve/spender",
            self.oneinch.base_url, self.chain_id
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .context("build http client")?;
        let v: Resp = client
            .get(url)
            .header("authorization", self.oneinch_auth_header()?)
            .send()
            .await
            .context("1inch spender request")?
            .error_for_status()
            .context("1inch spender status")?
            .json()
            .await
            .context("1inch spender json")?;
        Self::parse_address(&v.address)
    }

    pub async fn oneinch_swap_tx(
        &self,
        from: Address,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
        slippage_bps: u32,
    ) -> eyre::Result<(TransactionRequest, U256)> {
        #[derive(Debug, Deserialize)]
        struct Tx {
            to: String,
            data: String,
            value: String,
        }
        #[derive(Debug, Deserialize)]
        struct Resp {
            #[serde(rename = "toAmount")]
            to_amount: String,
            tx: Tx,
        }

        if !self.oneinch_base_url_is_allowed() {
            eyre::bail!("oneinch_base_url must use https (or http://localhost for local testing)");
        }

        // 1inch expects slippage in percent (e.g. 1.0 for 1%).
        let slip_pct = format!("{}.{:02}", slippage_bps / 100, slippage_bps % 100);
        let url = format!(
            "{}/{}/swap?src={token_in:#x}&dst={token_out:#x}&amount={amount_in}&from={from:#x}&slippage={slip_pct}",
            self.oneinch.base_url, self.chain_id
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .context("build http client")?;
        let resp: Resp = client
            .get(url)
            .header("authorization", self.oneinch_auth_header()?)
            .send()
            .await
            .context("1inch swap request")?
            .error_for_status()
            .context("1inch swap status")?
            .json()
            .await
            .context("1inch swap json")?;

        let to_amount = parse_u256_dec(&resp.to_amount).context("parse 1inch toAmount")?;
        let to = Self::parse_address(&resp.tx.to)?;
        let expected_router = Self::parse_address(ONEINCH_ROUTER)?;
        if to != expected_router {
            eyre::bail!("unexpected 1inch router: {to:#x}");
        }
        let data = resp.tx.data.strip_prefix("0x").unwrap_or(&resp.tx.data);
        let calldata = hex::decode(data).context("decode 1inch swap calldata")?;
        if calldata.is_empty() {
            eyre::bail!("1inch swap calldata is empty");
        }
        let value = parse_u256_dec(&resp.tx.value).context("parse 1inch value")?;
        let native_sentinel = Self::parse_address(ONEINCH_NATIVE_SENTINEL)?;
        if token_in == native_sentinel {
            if value != amount_in {
                eyre::bail!("unexpected 1inch tx value for native swap");
            }
        } else if !value.is_zero() {
            eyre::bail!("unexpected non-zero 1inch tx value");
        }

        let req = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_input(Bytes::from(calldata))
            .with_value(value);
        Ok((req, to_amount))
    }

    pub async fn quote_uniswap_exact_in(
        &self,
        token_in: Address,
        token_out: Address,
        amount_in: U256,
        fee: u32,
    ) -> eyre::Result<U256> {
        let Some(u) = &self.uniswap else {
            eyre::bail!("uniswap not configured for chain {}", self.name);
        };
        let uaddr = u.clone();

        self.with_fallback_and_backoff("uniswap quote", |p| async move {
            let q = IQuoterV2::new(uaddr.quoter_v2, &p);
            let params = IQuoterV2::QuoteExactInputSingleParams {
                tokenIn: token_in,
                tokenOut: token_out,
                amountIn: amount_in,
                fee: alloy::primitives::Uint::from(fee),
                sqrtPriceLimitX96: alloy::primitives::Uint::ZERO,
            };
            let out = q
                .quoteExactInputSingle(params)
                .call()
                .await
                .context("uniswap quote")?;
            Ok(out.amountOut)
        })
        .await
    }

    pub fn build_uniswap_swap_tx(
        &self,
        req: &UniswapSwapRequest,
    ) -> eyre::Result<TransactionRequest> {
        let Some(u) = &self.uniswap else {
            eyre::bail!("uniswap not configured for chain {}", self.name);
        };

        let provider = self.provider()?;
        let router = ISwapRouter02::new(u.router02, &provider);

        let recipient = if req.native_out { u.router02 } else { req.from };
        let params = ISwapRouter02::ExactInputSingleParams {
            tokenIn: req.token_in,
            tokenOut: req.token_out,
            fee: alloy::primitives::Uint::from(req.fee),
            recipient,
            amountIn: req.amount_in,
            amountOutMinimum: req.amount_out_min,
            sqrtPriceLimitX96: alloy::primitives::Uint::ZERO,
        };
        let exact = router.exactInputSingle(params);

        let mut value = U256::ZERO;
        let data: Bytes = if req.native_out {
            // exactInputSingle(recipient=router) then unwrapWETH9 to the user
            let unwrap = router.unwrapWETH9(U256::ZERO, req.from);
            let calldata_exact = exact.calldata().clone();
            let calldata_unwrap = unwrap.calldata().clone();
            let calls = vec![calldata_exact, calldata_unwrap];
            let mc = router.multicall(calls);
            mc.calldata().clone()
        } else {
            exact.calldata().clone()
        };

        if req.native_in {
            value = req.amount_in;
        }

        let mut tx = TransactionRequest::default()
            .with_from(req.from)
            .with_to(u.router02)
            .with_value(value)
            .with_input(data);
        tx.chain_id = Some(self.chain_id);
        Ok(tx)
    }
}

pub fn parse_u256_dec(s: &str) -> eyre::Result<U256> {
    s.trim().parse::<U256>().context("parse u256")
}

/// Extract the `to` address from a `TransactionRequest`.
pub fn extract_tx_to_address(tx: &TransactionRequest) -> eyre::Result<Address> {
    match tx.to {
        Some(TxKind::Call(a)) => Ok(a),
        _ => eyre::bail!("tx missing `to` address"),
    }
}

fn defaults_for(name: &str) -> (Vec<String>, Option<UniswapAddresses>) {
    let mut fallbacks = vec![];
    match name {
        "ethereum" => {
            fallbacks.push("https://cloudflare-eth.com".into());
        }
        "base" => {
            fallbacks.push("https://mainnet.base.org".into());
        }
        "arbitrum" => {
            fallbacks.push("https://arb1.arbitrum.io/rpc".into());
        }
        "optimism" => {
            fallbacks.push("https://mainnet.optimism.io".into());
        }
        "polygon" => {
            fallbacks.push("https://polygon-rpc.com".into());
        }
        "monad" => {
            fallbacks.push("https://rpc1.monad.xyz".into());
            fallbacks.push("https://rpc2.monad.xyz".into());
        }
        _ => {}
    }

    let uniswap = uniswap_defaults(name);

    (fallbacks, uniswap)
}

fn uniswap_defaults(name: &str) -> Option<UniswapAddresses> {
    fn addr(s: &str) -> Option<Address> {
        Address::from_str(s).ok()
    }

    match name {
        "ethereum" => Some(UniswapAddresses {
            router02: addr("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45")?,
            quoter_v2: addr("0x61fFE014bA17989E743c5F6cB21bF9697530B21e")?,
            wrapped_native: addr("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")?,
            usdc: addr("0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")?,
        }),
        "base" => Some(UniswapAddresses {
            router02: addr("0x2626664c2603336E57B271c5C0b26F421741e481")?,
            quoter_v2: addr("0x3d4e44Eb1374240CE5F1B871ab261CD16335B76a")?,
            wrapped_native: addr("0x4200000000000000000000000000000000000006")?,
            usdc: addr("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913")?,
        }),
        "arbitrum" => Some(UniswapAddresses {
            router02: addr("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45")?,
            quoter_v2: addr("0x61fFE014bA17989E743c5F6cB21bF9697530B21e")?,
            wrapped_native: addr("0x82aF49447D8a07e3bd95BD0d56f35241523fBab1")?,
            usdc: addr("0xaf88d065e77c8cC2239327C5EDb3A432268e5831")?,
        }),
        "optimism" => Some(UniswapAddresses {
            router02: addr("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45")?,
            quoter_v2: addr("0x61fFE014bA17989E743c5F6cB21bF9697530B21e")?,
            wrapped_native: addr("0x4200000000000000000000000000000000000006")?,
            usdc: addr("0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85")?,
        }),
        "polygon" => Some(UniswapAddresses {
            router02: addr("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45")?,
            quoter_v2: addr("0x61fFE014bA17989E743c5F6cB21bF9697530B21e")?,
            wrapped_native: addr("0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270")?,
            usdc: addr("0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174")?,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eip1559_fee_policy_is_conservative_and_monotonic() {
        let base_fee: u128 = 10_000_000_000; // 10 gwei
        let gas_price: u128 = 20_000_000_000; // 20 gwei
        let (max_fee, priority) = compute_eip1559_fees(base_fee, gas_price);
        // priority = max(1.5 gwei, gas_price/10 = 2 gwei)
        assert_eq!(priority, 2_000_000_000_u128, "priority mismatch");
        // max_fee = base_fee*2 + priority = 22 gwei
        assert_eq!(max_fee, 22_000_000_000_u128, "max_fee mismatch");
        assert!(
            max_fee >= base_fee + priority,
            "max_fee must be >= base + priority"
        );
    }

    #[test]
    fn eip1559_priority_has_min_floor() {
        let base_fee: u128 = 1_000_000_000; // 1 gwei
        let gas_price: u128 = 5_000_000_000; // 5 gwei -> /10 = 0.5 gwei
        let (_max_fee, priority) = compute_eip1559_fees(base_fee, gas_price);
        assert_eq!(priority, 1_500_000_000_u128, "priority should use floor");
    }

    #[test]
    fn apply_fee_policy_sets_eip1559_when_base_fee_present() {
        let from = Address::ZERO;
        let to = Address::ZERO;
        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_value(U256::from(1_u64));
        let base_fee = Some(10_000_000_000_u128);
        let gas_price = 20_000_000_000_u128;
        let out = apply_fee_policy(tx, base_fee, gas_price, from, 1);
        assert!(out.max_fee_per_gas.is_some(), "should set max_fee_per_gas");
        assert!(
            out.max_priority_fee_per_gas.is_some(),
            "should set max_priority_fee_per_gas"
        );
        assert!(out.gas_price.is_none(), "should not set legacy gas_price");
    }

    #[test]
    fn apply_fee_policy_sets_legacy_gas_price_when_base_fee_missing() {
        let from = Address::ZERO;
        let to = Address::ZERO;
        let tx = TransactionRequest::default().with_to(to);
        let out = apply_fee_policy(tx, None, 7, from, 1);
        assert_eq!(out.gas_price, Some(7_u128), "should set legacy gas_price");
        assert!(
            out.max_fee_per_gas.is_none(),
            "should not set eip1559 fields"
        );
    }
}

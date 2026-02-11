use alloy::{
    primitives::{keccak256, Address, B256, U256},
    signers::{local::PrivateKeySigner, SignerSync as _},
    sol,
    sol_types::{Eip712Domain, SolStruct as _},
};
use eyre::{Context as _, ContextCompat as _};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::time::Duration;

sol! {
    struct Agent {
        string source;
        bytes32 connectionId;
    }
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

fn utc_now_ms_u64() -> eyre::Result<u64> {
    let now_ms_i64 = chrono::Utc::now().timestamp_millis();
    u64::try_from(now_ms_i64).context("utc timestamp_millis is negative")
}

fn u256_hex32(v: U256) -> String {
    hex::encode(v.to_be_bytes::<32>())
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

fn host_prefix_ok(s: &str, prefix: &str) -> bool {
    if !s.starts_with(prefix) {
        return false;
    }
    matches!(s.as_bytes().get(prefix.len()), None | Some(b':' | b'/'))
}

fn is_loopback_http(url: &str) -> bool {
    let u = url.trim();
    host_prefix_ok(u, "http://127.0.0.1")
        || host_prefix_ok(u, "http://localhost")
        || host_prefix_ok(u, "http://[::1]")
}

fn base_url_is_allowed(url: &str) -> bool {
    let s = url.trim();
    if s.starts_with("https://") {
        return true;
    }
    if is_loopback_http(s) {
        return true;
    }
    allow_insecure_http()
}

#[derive(Debug, Clone)]
pub struct HyperliquidClient {
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HyperliquidMarket {
    pub asset: u32,
    pub coin: String,
    pub sz_decimals: u32,
    pub max_leverage: u32,
    pub only_isolated: bool,
    pub mid_px: Option<f64>,
    pub mark_px: Option<f64>,
    pub funding: Option<f64>,
    pub open_interest: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L1Signature {
    pub r: String,
    pub s: String,
    pub v: u64,
}

#[derive(Debug, Clone, Serialize)]
struct OrderTypeLimit<'a> {
    tif: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct OrderTypeWire<'a> {
    limit: OrderTypeLimit<'a>,
}

#[derive(Debug, Clone, Serialize)]
struct OrderWire<'a> {
    #[serde(rename = "a")]
    asset: u32,
    #[serde(rename = "b")]
    is_buy: bool,
    #[serde(rename = "p")]
    limit_px: &'a str,
    #[serde(rename = "s")]
    sz: &'a str,
    #[serde(rename = "r")]
    reduce_only: bool,
    #[serde(rename = "t")]
    order_type: OrderTypeWire<'a>,
    // Optional cloid ("c") omitted.
}

#[derive(Debug, Clone, Serialize)]
struct OrderAction<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    orders: Vec<OrderWire<'a>>,
    grouping: &'a str,
}

#[derive(Debug, Clone, Serialize)]
struct CancelWire {
    #[serde(rename = "a")]
    asset: u32,
    #[serde(rename = "o")]
    oid: u64,
}

#[derive(Debug, Clone, Serialize)]
struct CancelAction {
    #[serde(rename = "type")]
    ty: &'static str,
    cancels: Vec<CancelWire>,
}

#[derive(Debug, Clone, Serialize)]
struct UpdateLeverageAction {
    #[serde(rename = "type")]
    ty: &'static str,
    asset: u32,
    #[serde(rename = "isCross")]
    is_cross: bool,
    leverage: u32,
}

impl HyperliquidClient {
    pub fn new(base_url: &str) -> eyre::Result<Self> {
        if !base_url_is_allowed(base_url) {
            eyre::bail!("hyperliquid base_url must use https (or loopback)");
        }
        Ok(Self {
            base_url: base_url.trim().to_owned(),
        })
    }

    fn http() -> eyre::Result<Client> {
        Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .build()
            .context("build http client")
    }

    pub async fn info(&self, body: Value) -> eyre::Result<Value> {
        let url = format!("{}/info", self.base_url);
        let v: Value = Self::http()?
            .post(url)
            .json(&body)
            .send()
            .await
            .context("hyperliquid info request")?
            .error_for_status()
            .context("hyperliquid info status")?
            .json::<Value>()
            .await
            .context("hyperliquid info json")?;
        Ok(v)
    }

    pub async fn exchange(&self, body: Value) -> eyre::Result<Value> {
        let url = format!("{}/exchange", self.base_url);
        let v: Value = Self::http()?
            .post(url)
            .json(&body)
            .send()
            .await
            .context("hyperliquid exchange request")?
            .error_for_status()
            .context("hyperliquid exchange status")?
            .json::<Value>()
            .await
            .context("hyperliquid exchange json")?;
        Ok(v)
    }

    pub async fn meta_and_asset_ctxs(&self) -> eyre::Result<Vec<HyperliquidMarket>> {
        let v = self.info(json!({ "type": "metaAndAssetCtxs" })).await?;
        let arr = v.as_array().context("metaAndAssetCtxs must be array")?;
        let [meta_v, ctxs_v] = arr.as_slice() else {
            eyre::bail!("metaAndAssetCtxs: unexpected response shape");
        };
        let meta = meta_v.as_object().context("meta must be object")?;
        let universe = meta
            .get("universe")
            .and_then(|x| x.as_array())
            .context("meta.universe missing")?;
        let ctxs = ctxs_v.as_array().context("asset ctxs must be array")?;
        let mut out = vec![];
        for (i, u) in universe.iter().enumerate() {
            let uo = u.as_object().context("universe item must be object")?;
            let coin = uo
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned();

            let sz_decimals_u64 = uo.get("szDecimals").and_then(Value::as_u64).unwrap_or(0);
            let sz_decimals = u32::try_from(sz_decimals_u64).context("szDecimals out of range")?;

            let max_leverage_u64 = uo.get("maxLeverage").and_then(Value::as_u64).unwrap_or(0);
            let max_leverage =
                u32::try_from(max_leverage_u64).context("maxLeverage out of range")?;

            let only_isolated = uo
                .get("onlyIsolated")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let ctx = ctxs.get(i).and_then(|x| x.as_object());
            let mid_px = ctx
                .and_then(|c| c.get("midPx"))
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok());
            let mark_px = ctx
                .and_then(|c| c.get("markPx"))
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok());
            let funding = ctx
                .and_then(|c| c.get("funding"))
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok());
            let open_interest = ctx
                .and_then(|c| c.get("openInterest"))
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok());

            out.push(HyperliquidMarket {
                asset: u32::try_from(i).context("asset index out of range")?,
                coin,
                sz_decimals,
                max_leverage,
                only_isolated,
                mid_px,
                mark_px,
                funding,
                open_interest,
            });
        }
        Ok(out)
    }
}

/// Delegate to [`crate::financial_math::float_to_wire`].
pub fn float_to_wire(x: f64) -> eyre::Result<String> {
    crate::financial_math::float_to_wire(x)
}

/// Delegate to [`crate::financial_math::slippage_limit_px`].
pub fn slippage_limit_px(mid_px: f64, is_buy: bool, slippage: f64, sz_decimals: u32) -> f64 {
    crate::financial_math::slippage_limit_px(mid_px, is_buy, slippage, sz_decimals)
}

fn action_hash(
    msgpack_action: &[u8],
    vault_address: Option<Address>,
    nonce: u64,
    expires_after: Option<u64>,
) -> B256 {
    let mut data = Vec::with_capacity(msgpack_action.len() + 64);
    data.extend_from_slice(msgpack_action);
    data.extend_from_slice(&nonce.to_be_bytes());
    if let Some(a) = vault_address {
        data.push(1_u8);
        data.extend_from_slice(a.as_slice());
    } else {
        data.push(0_u8);
    }
    if let Some(ea) = expires_after {
        data.push(0_u8);
        data.extend_from_slice(&ea.to_be_bytes());
    }
    keccak256(data)
}

pub fn sign_l1_action<A>(
    signer: &PrivateKeySigner,
    action: &A,
    vault_address: Option<Address>,
    nonce_ms: u64,
    expires_after: Option<u64>,
    is_mainnet: bool,
) -> eyre::Result<L1Signature>
where
    A: Serialize + Sync,
{
    let packed = rmp_serde::to_vec_named(action).context("msgpack action")?;
    let h = action_hash(&packed, vault_address, nonce_ms, expires_after);
    let source = if is_mainnet { "a" } else { "b" };

    let domain = Eip712Domain {
        name: Some(Cow::Borrowed("Exchange")),
        version: Some(Cow::Borrowed("1")),
        chain_id: Some(U256::from(1337_u64)),
        verifying_contract: Some(Address::ZERO),
        salt: None,
    };

    let agent = Agent {
        source: source.to_owned(),
        connectionId: h,
    };

    let signing_hash = agent.eip712_signing_hash(&domain);
    let sig = signer
        .sign_hash_sync(&signing_hash)
        .context("sign typed data")?;
    Ok(L1Signature {
        r: format!("0x{}", u256_hex32(sig.r())),
        s: format!("0x{}", u256_hex32(sig.s())),
        v: u64::from(sig.v()) + 27,
    })
}

/// Common session parameters shared by all Hyperliquid exchange requests.
pub struct SessionParams<'a> {
    pub client: &'a HyperliquidClient,
    pub wallet: &'a PrivateKeySigner,
    pub is_mainnet: bool,
    pub vault_address: Option<Address>,
    pub expires_after: Option<u64>,
}

/// Parameters for placing a Hyperliquid order.
pub struct OrderParams<'a> {
    pub asset: u32,
    pub is_buy: bool,
    pub sz: &'a str,
    pub limit_px: &'a str,
    pub reduce_only: bool,
    pub tif: &'a str,
}

/// Parameters for updating Hyperliquid leverage.
pub struct LeverageParams {
    pub asset: u32,
    pub leverage: u32,
    pub is_cross: bool,
}

/// Post an order using structured params (preferred for new call sites).
pub async fn post_order_params(
    session: &SessionParams<'_>,
    order: &OrderParams<'_>,
) -> eyre::Result<Value> {
    let action = OrderAction {
        ty: "order",
        orders: vec![OrderWire {
            asset: order.asset,
            is_buy: order.is_buy,
            limit_px: order.limit_px,
            sz: order.sz,
            reduce_only: order.reduce_only,
            order_type: OrderTypeWire {
                limit: OrderTypeLimit { tif: order.tif },
            },
        }],
        grouping: "na",
    };
    submit_exchange(session, &action).await
}

/// Update leverage using structured params (preferred for new call sites).
pub async fn post_update_leverage_params(
    session: &SessionParams<'_>,
    params: &LeverageParams,
) -> eyre::Result<Value> {
    let action = UpdateLeverageAction {
        ty: "updateLeverage",
        asset: params.asset,
        is_cross: params.is_cross,
        leverage: params.leverage,
    };
    submit_exchange(session, &action).await
}

/// Sign and submit a Hyperliquid exchange action.
async fn submit_exchange<A: Serialize + Sync>(
    session: &SessionParams<'_>,
    action: &A,
) -> eyre::Result<Value> {
    let now_ms = utc_now_ms_u64()?;
    let sig = sign_l1_action(
        session.wallet,
        action,
        session.vault_address,
        now_ms,
        session.expires_after,
        session.is_mainnet,
    )?;
    session
        .client
        .exchange(json!({
          "action": action,
          "nonce": now_ms,
          "signature": sig,
          "vaultAddress": session.vault_address.map(|a| format!("{a:#x}")),
          "expiresAfter": session.expires_after
        }))
        .await
}

/// Legacy positional-args API (kept for existing callers in tools/perps.rs).
pub async fn post_order(
    session: &SessionParams<'_>,
    order: &OrderParams<'_>,
) -> eyre::Result<Value> {
    post_order_params(session, order).await
}

/// Legacy positional-args API (kept for existing callers in tools/perps.rs).
pub async fn post_update_leverage(
    session: &SessionParams<'_>,
    params: &LeverageParams,
) -> eyre::Result<Value> {
    post_update_leverage_params(session, params).await
}

pub async fn post_cancel(
    client: &HyperliquidClient,
    wallet: &PrivateKeySigner,
    is_mainnet: bool,
    vault_address: Option<Address>,
    expires_after: Option<u64>,
    asset: u32,
    oid: u64,
) -> eyre::Result<Value> {
    let action = CancelAction {
        ty: "cancel",
        cancels: vec![CancelWire { asset, oid }],
    };
    let now_ms = utc_now_ms_u64()?;
    let sig = sign_l1_action(
        wallet,
        &action,
        vault_address,
        now_ms,
        expires_after,
        is_mainnet,
    )?;
    client
        .exchange(json!({
          "action": action,
          "nonce": now_ms,
          "signature": sig,
          "vaultAddress": vault_address.map(|a| format!("{a:#x}")),
          "expiresAfter": expires_after
        }))
        .await
}

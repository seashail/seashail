use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PolicyBool(bool);

impl PolicyBool {
    pub const fn new(v: bool) -> Self {
        Self(v)
    }

    pub const fn get(self) -> bool {
        self.0
    }
}

impl From<bool> for PolicyBool {
    fn from(v: bool) -> Self {
        Self::new(v)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    /// Auto-approve below this USD amount.
    pub auto_approve_usd: f64,
    /// Require user confirmation (elicitation) up to this USD amount.
    pub confirm_up_to_usd: f64,
    /// Hard-block above this USD amount.
    pub hard_block_over_usd: f64,

    /// Hard per-transaction USD limit (independent of tiering).
    pub max_usd_per_tx: f64,
    /// Daily (UTC) aggregate USD limit across write ops.
    pub max_usd_per_day: f64,

    /// Maximum allowed slippage for swaps (basis points).
    pub max_slippage_bps: u32,

    /// Deny any write operation where Seashail cannot compute a USD value (pricing unavailable).
    ///
    /// Security posture: fail closed. This prevents "`usd_value=0`" from turning into auto-approval.
    pub deny_unknown_usd_value: PolicyBool,

    /// Require explicit user confirmation for any transaction whose bytes were constructed remotely
    /// (e.g. aggregator "swap transaction" APIs).
    ///
    /// This is independent of USD tiering and is intended to mitigate "remote tx construction"
    /// risk. (Seashail still applies allowlists where applicable.)
    pub require_user_confirm_for_remote_tx: PolicyBool,

    /// Operation toggles.
    pub enable_send: PolicyBool,
    pub enable_swap: PolicyBool,
    pub enable_perps: PolicyBool,
    pub enable_nft: PolicyBool,
    /// pump.fun operations.
    pub enable_pumpfun: PolicyBool,
    /// Cross-chain bridging operations.
    pub enable_bridge: PolicyBool,
    /// Lending/borrowing operations.
    pub enable_lending: PolicyBool,
    /// Staking/yield operations.
    pub enable_staking: PolicyBool,
    /// Liquidity provision operations.
    pub enable_liquidity: PolicyBool,
    /// Prediction market operations.
    pub enable_prediction: PolicyBool,

    /// OFAC SDN blocking. If enabled, Seashail blocks transactions to listed addresses.
    ///
    /// Users can disable this if it is not applicable to their jurisdiction.
    pub enable_ofac_sdn: PolicyBool,

    /// Treat internal transfers (between Seashail-managed wallets/accounts) as policy-exempt.
    ///
    /// When true (default), internal transfers bypass tiered approval and USD caps because they
    /// cannot exfiltrate funds to an external recipient. Hard blocks still apply (scam blocklist,
    /// OFAC when enabled).
    ///
    /// When false, internal transfers are still allowed, but they are evaluated against the normal
    /// tiering and USD caps (and will count toward daily limits).
    pub internal_transfers_exempt: PolicyBool,

    /// Allow sending to any address (disables allowlisting).
    pub send_allow_any: PolicyBool,
    /// For token transfers, only allow sending to these addresses by default.
    /// If empty and `send_allow_any == false`, all sends are blocked.
    pub send_allowlist: Vec<String>,

    /// Allow `DeFi` contract interactions with any contract (disables allowlisting).
    pub contract_allow_any: PolicyBool,
    /// For `DeFi` interactions, restrict to known contract addresses.
    /// If empty and `contract_allow_any == false`, Seashail enforces a built-in
    /// allowlist for known protocol routers (recommended).
    pub contract_allowlist: Vec<String>,

    /// Perpetuals risk controls.
    pub max_leverage: u32,
    pub max_usd_per_position: f64,

    /// NFT risk controls.
    pub max_usd_per_nft_tx: f64,

    /// pump.fun risk controls.
    ///
    /// NOTE: These are chain-native units (SOL), not USD. They are enforced in the pump.fun tool
    /// handlers in addition to the global USD caps above.
    pub pumpfun_max_sol_per_buy: f64,
    /// Maximum number of pump.fun buys per hour (rolling window) per wallet+account.
    pub pumpfun_max_buys_per_hour: u32,

    /// Bridge/lending/staking/liquidity/prediction caps.
    pub max_usd_per_bridge_tx: f64,
    pub max_usd_per_lending_tx: f64,
    pub max_usd_per_stake_tx: f64,
    pub max_usd_per_liquidity_tx: f64,
    pub max_usd_per_prediction_tx: f64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            auto_approve_usd: 10.0,
            confirm_up_to_usd: 1_000.0,
            hard_block_over_usd: 1_000.0,

            max_usd_per_tx: 100.0,
            max_usd_per_day: 500.0,

            max_slippage_bps: 100, // 1.0%

            deny_unknown_usd_value: true.into(),
            require_user_confirm_for_remote_tx: true.into(),

            enable_send: true.into(),
            enable_swap: true.into(),
            enable_perps: true.into(),
            enable_nft: true.into(),
            enable_pumpfun: true.into(),
            enable_bridge: true.into(),
            enable_lending: true.into(),
            enable_staking: true.into(),
            enable_liquidity: true.into(),
            enable_prediction: true.into(),
            enable_ofac_sdn: true.into(),

            internal_transfers_exempt: true.into(),

            send_allow_any: false.into(),
            send_allowlist: vec![],
            contract_allow_any: false.into(),
            contract_allowlist: vec![],

            max_leverage: 3,
            max_usd_per_position: 100.0,
            max_usd_per_nft_tx: 100.0,

            pumpfun_max_sol_per_buy: 0.1,
            pumpfun_max_buys_per_hour: 10,

            max_usd_per_bridge_tx: 100.0,
            max_usd_per_lending_tx: 200.0,
            max_usd_per_stake_tx: 500.0,
            max_usd_per_liquidity_tx: 100.0,
            max_usd_per_prediction_tx: 100.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_enables_extended_surfaces() {
        let p = Policy::default();
        assert!(p.enable_pumpfun.get());
        assert!(p.enable_bridge.get());
        assert!(p.enable_lending.get());
        assert!(p.enable_staking.get());
        assert!(p.enable_liquidity.get());
        assert!(p.enable_prediction.get());
    }
}

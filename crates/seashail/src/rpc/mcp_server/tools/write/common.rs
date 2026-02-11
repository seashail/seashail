use crate::chains::evm::EvmChain;
use alloy::primitives::{Address, U256};
use tokio::time::{sleep, Duration};

pub(super) fn summarize_sim_error(e: &eyre::Report, label: &str) -> String {
    let s = format!("{e:#}").to_lowercase();
    if s.contains("insufficient funds") {
        return "simulation failed: insufficient funds".to_owned();
    }
    if s.contains("intrinsic gas too low") {
        return "simulation failed: intrinsic gas too low".to_owned();
    }
    if s.contains("execution reverted") || s.contains("revert") {
        return "simulation failed: execution reverted".to_owned();
    }
    format!("simulation failed ({label})")
}

pub(super) async fn wait_for_allowance(
    evm: &EvmChain,
    token: Address,
    owner: Address,
    spender: Address,
    min_allowance: U256,
) -> bool {
    for _ in 0_u32..120_u32 {
        let Ok(a) = evm.erc20_allowance(token, owner, spender).await else {
            sleep(Duration::from_millis(250)).await;
            continue;
        };
        if a >= min_allowance {
            return true;
        }
        sleep(Duration::from_millis(250)).await;
    }
    false
}

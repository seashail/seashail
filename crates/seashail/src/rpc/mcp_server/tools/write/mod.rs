mod aave;
mod common;
mod compound;
mod defi_tx_envelope;
mod fund_wallets;
mod kamino;
mod marginfi;
mod polymarket;
mod pumpfun;
mod request_airdrop;
mod send_transaction;
mod staking;
mod swap_tokens;
mod transfer_between_wallets;
mod wormhole;
mod wormhole_solana;

use serde_json::Value;
use tokio::io::BufReader;

use super::super::jsonrpc::{err, JsonRpcResponse};
use super::super::{ConnState, SharedState};

pub struct HandlerCtx<'a, R, W> {
    pub req_id: Value,
    pub args: Value,
    pub shared: &'a mut SharedState,
    pub conn: &'a mut ConnState,
    pub stdin: &'a mut tokio::io::Lines<BufReader<R>>,
    pub stdout: &'a mut W,
}

fn arg_str_trimmed<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("").trim()
}

fn has_nonempty_str(args: &Value, key: &str) -> bool {
    args.get(key)
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

async fn route_bridge<R, W>(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let chain = arg_str_trimmed(&args, "chain");
    let provider = args
        .get("bridge_provider")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("provider").and_then(|v| v.as_str()))
        .unwrap_or("wormhole")
        .trim();
    let has_native = has_nonempty_str(&args, "to_chain")
        && has_nonempty_str(&args, "token")
        && has_nonempty_str(&args, "amount");

    if provider == "wormhole" && has_native {
        if chain == "solana" {
            wormhole_solana::handle(req_id, args, shared, conn, stdin, stdout).await
        } else {
            let mut ctx = HandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            wormhole::handle(&mut ctx).await
        }
    } else {
        defi_tx_envelope::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
    }
}

async fn route_lending<R, W>(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let chain = arg_str_trimmed(&args, "chain");
    let protocol = {
        let p = arg_str_trimmed(&args, "protocol");
        let p = if p.is_empty() { "auto" } else { p };
        if p == "auto" {
            if chain == "solana" {
                "kamino"
            } else {
                "aave"
            }
        } else {
            p
        }
    };
    let has_native = has_nonempty_str(&args, "token") && has_nonempty_str(&args, "amount");

    if chain == "solana" && protocol == "kamino" && has_native {
        kamino::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
    } else if chain == "solana" && protocol == "marginfi" && has_native {
        let mut ctx = HandlerCtx {
            req_id,
            args,
            shared,
            conn,
            stdin,
            stdout,
        };
        marginfi::handle(tool_name, &mut ctx).await
    } else if chain != "solana" && protocol == "aave" && has_native {
        let mut ctx = HandlerCtx {
            req_id,
            args,
            shared,
            conn,
            stdin,
            stdout,
        };
        aave::handle(tool_name, &mut ctx).await
    } else if chain != "solana" && protocol == "compound" && has_native {
        compound::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
    } else {
        defi_tx_envelope::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
    }
}

async fn route_staking<R, W>(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let chain = arg_str_trimmed(&args, "chain");
    let protocol = {
        let p = arg_str_trimmed(&args, "protocol");
        if p.is_empty() {
            if chain == "solana" {
                "jito"
            } else {
                "lido"
            }
        } else {
            p
        }
    };
    let has_native = has_nonempty_str(&args, "amount");

    if matches!((chain, protocol), ("solana", "jito") | ("ethereum", "lido")) && has_native {
        let mut ctx = HandlerCtx {
            req_id,
            args,
            shared,
            conn,
            stdin,
            stdout,
        };
        staking::handle(tool_name, &mut ctx).await
    } else {
        defi_tx_envelope::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
    }
}

pub async fn handle<R, W>(
    req_id: Value,
    tool_name: &str,
    args: Value,
    shared: &mut SharedState,
    conn: &mut ConnState,
    stdin: &mut tokio::io::Lines<BufReader<R>>,
    stdout: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    match tool_name {
        "request_airdrop" => request_airdrop::handle(req_id, args, shared, conn).await,
        "send_transaction" => {
            let mut ctx = HandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            send_transaction::handle_ctx(&mut ctx).await
        }
        "swap_tokens" => {
            let mut ctx = HandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            swap_tokens::handle_ctx(&mut ctx).await
        }
        "transfer_between_wallets" => {
            let mut ctx = HandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            transfer_between_wallets::handle_ctx(&mut ctx).await
        }
        "fund_wallets" => {
            let mut ctx = HandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            fund_wallets::handle(&mut ctx).await
        }
        "pumpfun_buy" | "pumpfun_sell" => {
            let mut ctx = HandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            pumpfun::handle(tool_name, &mut ctx).await
        }
        "place_prediction" | "close_prediction" => {
            let mut ctx = HandlerCtx {
                req_id,
                args,
                shared,
                conn,
                stdin,
                stdout,
            };
            polymarket::handle(tool_name, &mut ctx).await
        }
        "bridge_tokens" => route_bridge(req_id, tool_name, args, shared, conn, stdin, stdout).await,
        "lend_tokens" | "withdraw_lending" | "borrow_tokens" | "repay_borrow" => {
            route_lending(req_id, tool_name, args, shared, conn, stdin, stdout).await
        }
        "stake_tokens" | "unstake_tokens" => {
            route_staking(req_id, tool_name, args, shared, conn, stdin, stdout).await
        }
        "provide_liquidity" | "remove_liquidity" => {
            defi_tx_envelope::handle(req_id, tool_name, args, shared, conn, stdin, stdout).await
        }
        _ => Ok(err(req_id, -32601, "unknown tool")),
    }
}

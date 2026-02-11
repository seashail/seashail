//! Centralised helpers for user-facing CLI output written to stderr.

use std::io::{BufRead as _, IsTerminal as _, Write as _};

fn stderr_write(s: &str) {
    let mut stderr = std::io::stderr().lock();
    if stderr.write_all(s.as_bytes()).is_err() {
        return;
    }
    let _flush = stderr.flush();
}

fn stderr_writeln(s: &str) {
    let mut stderr = std::io::stderr().lock();
    if stderr.write_all(s.as_bytes()).is_err() {
        return;
    }
    if stderr.write_all(b"\n").is_err() {
        return;
    }
    let _flush = stderr.flush();
}

/// Print the MCP startup banner to stderr (human-operator info only).
///
/// `network` is a display-friendly string like `"mainnet"`, `"testnet"`, or `"auto"`.
/// `mode` is `"standalone"` or `"proxy"`.
pub fn print_mcp_banner(version: &str, network: &str, mode: &str) {
    stderr_writeln(&format!(
        "Seashail MCP\n============\nVersion : v{version}\nNetwork : {network}\nMode    : {mode} (stdio)\n\nTip: if your agent can't connect, run `seashail doctor`."
    ));
}

/// Print a wallet-created notice to stderr (human-operator info only).
pub fn print_wallet_created(name: &str) {
    stderr_writeln(&format!(
        "Seashail: created default wallet '{name}' (EVM/Solana/Bitcoin addresses ready)."
    ));
}

/// Prompt the user on stderr to confirm an upgrade, or bail if non-interactive.
pub fn confirm_upgrade_or_bail(yes: bool) -> eyre::Result<()> {
    if yes {
        return Ok(());
    }
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !interactive {
        eyre::bail!("refusing to run upgrade non-interactively; pass --yes");
    }

    stderr_writeln(
        "Seashail upgrade will run the hosted installer to reinstall the latest Seashail.",
    );
    stderr_write("Continue? [y/N] ");
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| eyre::eyre!("read confirmation: {e}"))?;
    let ans = line.trim().to_ascii_lowercase();
    if ans == "y" || ans == "yes" {
        Ok(())
    } else {
        eyre::bail!("upgrade cancelled")
    }
}

use crate::{config::SeashailConfig, paths::SeashailPaths, wallet::WalletStore};
use eyre::Context as _;
use serde_json::json;
use std::{fs, path::Path, path::PathBuf};

#[cfg(unix)]
use tokio::net::UnixStream;

#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions;

#[cfg(not(any(unix, windows)))]
use tokio::net::TcpStream;

#[cfg(windows)]
fn pipe_name(paths: &SeashailPaths) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    paths.data_dir.to_string_lossy().hash(&mut h);
    format!(r"\\.\pipe\seashail-mcp-{:016x}", h.finish())
}

fn config_toml_path(paths: &SeashailPaths) -> PathBuf {
    paths.config_dir.join("config.toml")
}

fn wallet_index_path(paths: &SeashailPaths) -> PathBuf {
    paths.config_dir.join("wallets").join("index.json")
}

fn daemon_lock_path(paths: &SeashailPaths) -> PathBuf {
    paths.data_dir.join("seashail-daemon.lock")
}

#[cfg(unix)]
fn daemon_transport_label(paths: &SeashailPaths) -> (String, String) {
    (
        "unix_socket".to_owned(),
        paths
            .data_dir
            .join("seashail-mcp.sock")
            .to_string_lossy()
            .to_string(),
    )
}

#[cfg(windows)]
fn daemon_transport_label(paths: &SeashailPaths) -> (String, String) {
    ("named_pipe".to_owned(), pipe_name(paths))
}

#[cfg(not(any(unix, windows)))]
fn daemon_transport_label(_paths: &SeashailPaths) -> (String, String) {
    ("tcp_loopback".to_owned(), "127.0.0.1:41777".to_owned())
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

async fn probe_daemon_running(paths: &SeashailPaths) -> (bool, Option<String>) {
    // Best-effort: attempt a raw transport connect. We don't send any RPC here.
    // If it fails, daemon may simply be idle-stopped; `seashail mcp` will autostart.
    let timeout = std::time::Duration::from_millis(250);

    #[cfg(unix)]
    {
        let sock = paths.data_dir.join("seashail-mcp.sock");
        let fut = UnixStream::connect(&sock);
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(_s)) => (true, None),
            Ok(Err(e)) => (false, Some(format!("connect failed: {e}"))),
            Err(_) => (false, Some("connect timed out".to_owned())),
        }
    }

    #[cfg(windows)]
    {
        let name = pipe_name(paths);
        // Named pipe open is sync; keep it best-effort and quick.
        match ClientOptions::new().open(&name) {
            Ok(_c) => (true, None),
            Err(e) => (false, Some(format!("open failed: {e}"))),
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let fut = TcpStream::connect("127.0.0.1:41777");
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(_s)) => (true, None),
            Ok(Err(e)) => (false, Some(format!("connect failed: {e}"))),
            Err(_) => (false, Some("connect timed out".to_owned())),
        }
    }
}

fn try_parse_config(path: &Path) -> eyre::Result<SeashailConfig> {
    let s = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let cfg: SeashailConfig = toml::from_str(&s).context("parse config.toml")?;
    Ok(cfg)
}

struct PathsReport {
    config_dir: PathBuf,
    data_dir: PathBuf,
    log_file: PathBuf,
}

struct ConfigReport {
    path: PathBuf,
    exists: bool,
    parse_ok: bool,
    error: Option<String>,
    network_mode_configured: Option<String>,
    network_mode_effective: Option<String>,
    solana_rpc_url: Option<String>,
    evm_chain_count: usize,
    scam_blocklist_configured: Option<bool>,
    scam_blocklist_pubkey_pinned: Option<bool>,
}

struct WalletsReport {
    index_path: PathBuf,
    index_exists: bool,
    count: usize,
}

struct DaemonReport {
    lock_path: PathBuf,
    lock_exists: bool,
    transport: String,
    address: String,
    reachable: bool,
    note: Option<String>,
}

struct DoctorReport {
    version: &'static str,
    paths: PathsReport,
    config: ConfigReport,
    wallets: WalletsReport,
    daemon: DaemonReport,
    env: serde_json::Value,
}

async fn collect(paths: &SeashailPaths) -> eyre::Result<DoctorReport> {
    let config_path = config_toml_path(paths);
    let config_exists = config_path.exists();
    let (config_ok, config_err, cfg) = if config_exists {
        match try_parse_config(&config_path) {
            Ok(cfg) => (true, None, Some(cfg)),
            Err(e) => (false, Some(format!("{e:#}")), None),
        }
    } else {
        (false, None, None)
    };

    let wallet_store = WalletStore::new(paths);
    let wallet_count = wallet_store.list().unwrap_or_default().len();
    let wallet_index_path = wallet_index_path(paths);
    let wallet_index_exists = wallet_index_path.exists();

    let (daemon_transport, daemon_address) = daemon_transport_label(paths);
    let daemon_lock_path = daemon_lock_path(paths);
    let daemon_lock_exists = daemon_lock_path.exists();
    let (daemon_reachable, daemon_note) = probe_daemon_running(paths).await;

    let network_mode_effective = cfg
        .as_ref()
        .map(SeashailConfig::effective_network_mode)
        .map(|m| format!("{m:?}").to_lowercase());
    let network_mode_configured = cfg
        .as_ref()
        .and_then(|c| c.network_mode)
        .map(|m| format!("{m:?}").to_lowercase());

    let solana_rpc_url = cfg.as_ref().map(|c| c.rpc.solana_rpc_url.clone());
    let evm_chain_count = cfg.as_ref().map_or(0, |c| c.rpc.evm_rpc_urls.len());
    let scam_blocklist_configured = cfg.as_ref().map(|c| {
        c.http
            .scam_blocklist_url
            .as_ref()
            .is_some_and(|u| !u.trim().is_empty())
    });
    let scam_blocklist_pubkey_pinned = cfg.as_ref().map(|c| {
        c.http
            .scam_blocklist_pubkey_b64
            .as_ref()
            .is_some_and(|k| !k.trim().is_empty())
    });

    let env = json!({
      "SEASHAIL_CONFIG_DIR": env_opt("SEASHAIL_CONFIG_DIR"),
      "SEASHAIL_DATA_DIR": env_opt("SEASHAIL_DATA_DIR"),
      "SEASHAIL_NETWORK_MODE": env_opt("SEASHAIL_NETWORK_MODE"),
      "SEASHAIL_NETWORK": env_opt("SEASHAIL_NETWORK"),
      "SEASHAIL_TESTNET_MODE": env_opt("SEASHAIL_TESTNET_MODE"),
      "SEASHAIL_PASSPHRASE_set": std::env::var("SEASHAIL_PASSPHRASE").is_ok(),
    });

    Ok(DoctorReport {
        version: env!("CARGO_PKG_VERSION"),
        paths: PathsReport {
            config_dir: paths.config_dir.clone(),
            data_dir: paths.data_dir.clone(),
            log_file: paths.log_file.clone(),
        },
        config: ConfigReport {
            path: config_path,
            exists: config_exists,
            parse_ok: config_ok,
            error: config_err,
            network_mode_configured,
            network_mode_effective,
            solana_rpc_url,
            evm_chain_count,
            scam_blocklist_configured,
            scam_blocklist_pubkey_pinned,
        },
        wallets: WalletsReport {
            index_path: wallet_index_path,
            index_exists: wallet_index_exists,
            count: wallet_count,
        },
        daemon: DaemonReport {
            lock_path: daemon_lock_path,
            lock_exists: daemon_lock_exists,
            transport: daemon_transport,
            address: daemon_address,
            reachable: daemon_reachable,
            note: daemon_note,
        },
        env,
    })
}

fn print_json(out: &mut impl std::io::Write, r: &DoctorReport) -> eyre::Result<()> {
    let s = serde_json::to_string_pretty(&json!({
      "ok": true,
      "version": r.version,
      "paths": {
        "config_dir": r.paths.config_dir,
        "data_dir": r.paths.data_dir,
        "log_file": r.paths.log_file,
      },
      "config": {
        "path": r.config.path,
        "exists": r.config.exists,
        "parse_ok": r.config.parse_ok,
        "error": r.config.error,
        "network_mode": {
          "configured": r.config.network_mode_configured,
          "effective": r.config.network_mode_effective,
        },
        "rpc": {
          "solana_rpc_url": r.config.solana_rpc_url,
          "evm_chain_count": r.config.evm_chain_count,
        },
        "scam_blocklist": {
          "opt_in": true,
          "configured": r.config.scam_blocklist_configured,
          "pubkey_pinned": r.config.scam_blocklist_pubkey_pinned
        }
      },
      "wallets": {
        "index_path": r.wallets.index_path,
        "index_exists": r.wallets.index_exists,
        "count": r.wallets.count,
      },
      "daemon": {
        "lock_path": r.daemon.lock_path,
        "lock_exists": r.daemon.lock_exists,
        "transport": r.daemon.transport,
        "address": r.daemon.address,
        "reachable": r.daemon.reachable,
        "note": r.daemon.note,
      },
      "env": r.env,
      "hints": [
        "If your agent can't connect, install an integration (OpenClaw/Claude/Codex) that runs: seashail mcp",
        "If wallets.count == 0, connect via MCP and call list_wallets (or any wallet tool). Seashail will auto-create a machine-local default wallet. For portability/recovery, call export_shares/rotate_shares.",
        "Scam blocklist is opt-in. If you want it, configure http.scam_blocklist_url (and pin http.scam_blocklist_pubkey_b64).",
      ]
    }))
    .context("serialize doctor json")?;
    writeln!(out, "{s}").context("write doctor json")?;
    Ok(())
}

fn print_human(out: &mut impl std::io::Write, r: &DoctorReport) -> eyre::Result<()> {
    writeln!(out, "Seashail doctor (v{})", r.version).context("write header")?;
    writeln!(out).context("write newline")?;

    writeln!(out, "Paths:").context("write paths header")?;
    writeln!(out, "  config_dir: {}", r.paths.config_dir.display()).context("write paths")?;
    writeln!(out, "  data_dir:   {}", r.paths.data_dir.display()).context("write paths")?;
    writeln!(out, "  log_file:   {}", r.paths.log_file.display()).context("write paths")?;
    writeln!(out).context("write newline")?;

    writeln!(out, "Config:").context("write config header")?;
    writeln!(out, "  config.toml: {}", r.config.path.display()).context("write config")?;
    if !r.config.exists {
        writeln!(out, "  status: missing (will be created on first run)")
            .context("write config")?;
    } else if r.config.parse_ok {
        writeln!(
            out,
            "  status: ok (network_mode: configured={:?}, effective={:?})",
            r.config.network_mode_configured, r.config.network_mode_effective
        )
        .context("write config")?;
    } else {
        writeln!(out, "  status: parse failed").context("write config")?;
        if let Some(e) = &r.config.error {
            let first = e.lines().next().unwrap_or("parse error");
            writeln!(out, "  error: {first}").context("write config")?;
        }
    }
    writeln!(out).context("write newline")?;

    writeln!(out, "Scam Blocklist (opt-in):").context("write blocklist header")?;
    match (
        r.config.scam_blocklist_configured,
        r.config.scam_blocklist_pubkey_pinned,
    ) {
        (Some(true), Some(pinned)) => {
            writeln!(out, "  enabled: true").context("write blocklist")?;
            writeln!(out, "  pubkey_pinned: {pinned}").context("write blocklist")?;
        }
        (Some(false), _) => {
            writeln!(
                out,
                "  enabled: false (configure http.scam_blocklist_url to enable)"
            )
            .context("write blocklist")?;
        }
        _ => {
            writeln!(out, "  status: unknown (config missing or parse failed)")
                .context("write blocklist")?;
        }
    }
    writeln!(out).context("write newline")?;

    writeln!(out, "Wallets:").context("write wallets header")?;
    writeln!(out, "  index.json: {}", r.wallets.index_path.display()).context("write wallets")?;
    writeln!(out, "  index_exists: {}", r.wallets.index_exists).context("write wallets")?;
    writeln!(out, "  wallet_count: {}", r.wallets.count).context("write wallets")?;
    writeln!(out).context("write newline")?;

    writeln!(out, "Daemon:").context("write daemon header")?;
    writeln!(out, "  lock_path: {}", r.daemon.lock_path.display()).context("write daemon")?;
    writeln!(out, "  lock_exists: {}", r.daemon.lock_exists).context("write daemon")?;
    writeln!(out, "  transport: {}", r.daemon.transport).context("write daemon")?;
    writeln!(out, "  address: {}", r.daemon.address).context("write daemon")?;
    writeln!(out, "  reachable: {}", r.daemon.reachable).context("write daemon")?;
    if let Some(note) = &r.daemon.note {
        writeln!(out, "  note: {note}").context("write daemon")?;
    }
    writeln!(out).context("write newline")?;

    writeln!(out, "Env (redacted):").context("write env header")?;
    writeln!(
        out,
        "  SEASHAIL_CONFIG_DIR: {:?}",
        r.env.get("SEASHAIL_CONFIG_DIR").and_then(|v| v.as_str())
    )
    .context("write env")?;
    writeln!(
        out,
        "  SEASHAIL_DATA_DIR:   {:?}",
        r.env.get("SEASHAIL_DATA_DIR").and_then(|v| v.as_str())
    )
    .context("write env")?;
    writeln!(
        out,
        "  SEASHAIL_NETWORK_MODE: {:?}",
        r.env.get("SEASHAIL_NETWORK_MODE").and_then(|v| v.as_str())
    )
    .context("write env")?;
    writeln!(
        out,
        "  SEASHAIL_PASSPHRASE_set: {}",
        r.env
            .get("SEASHAIL_PASSPHRASE_set")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    )
    .context("write env")?;
    Ok(())
}

pub async fn run(as_json: bool) -> eyre::Result<()> {
    let paths = SeashailPaths::discover()?;
    let report = collect(&paths).await.context("collect doctor report")?;
    let mut out = std::io::stdout().lock();
    if as_json {
        print_json(&mut out, &report)?;
    } else {
        print_human(&mut out, &report)?;
    }
    Ok(())
}

use eyre::{Context as _, ContextCompat as _};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

use crate::agent::AgentNetwork;
use crate::keystore::{utc_now_iso, Keystore};
use crate::paths::SeashailPaths;

fn ensure_default_wallet_cli() -> eyre::Result<()> {
    let paths = SeashailPaths::discover().context("discover seashail paths")?;
    let ks = Keystore::open(paths).context("open keystore")?;
    if !ks.list_wallets().context("list wallets")?.is_empty() {
        return Ok(());
    }

    let lock = ks.acquire_write_lock().context("acquire keystore lock")?;
    if !ks.list_wallets().context("list wallets")?.is_empty() {
        Keystore::release_lock(lock)?;
        return Ok(());
    }

    let info = ks
        .create_generated_wallet_machine_only("default".to_owned())
        .context("create generated wallet")?;
    let _set_active_wallet = ks.set_active_wallet(&info.name, 0);

    // Record a minimal history entry.
    ks.append_tx_history(&json!({
      "ts": utc_now_iso(),
      "day": Keystore::current_utc_day_key(),
      "type": "wallet_created",
      "wallet": info.name,
      "wallet_kind": "generated",
      "source": "openclaw_install"
    }))
    .ok();

    Keystore::release_lock(lock)?;

    crate::cli_output::print_wallet_created(&info.name);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct InstallFlag(bool);

impl InstallFlag {
    pub const fn new(v: bool) -> Self {
        Self(v)
    }

    pub const fn get(self) -> bool {
        self.0
    }
}

impl From<bool> for InstallFlag {
    fn from(v: bool) -> Self {
        Self::new(v)
    }
}

#[derive(Debug, Clone)]
pub struct InstallFlags {
    /// If true and plugin is a local path, use `openclaw plugins install -l`.
    pub link: InstallFlag,
    /// Restart the `OpenClaw` gateway service after installation/config update.
    pub restart_gateway: InstallFlag,
    /// If true, add "seashail" to `OpenClaw`'s sandbox allowlist at tools.sandbox.tools.allow.
    pub enable_in_sandbox: InstallFlag,
    /// If true, ensure Seashail has a default wallet after installation.
    pub onboard_wallet: InstallFlag,
}

#[derive(Debug, Clone)]
pub struct InstallOpts {
    pub network: AgentNetwork,
    /// Path to openclaw.json (defaults to $`OPENCLAW_CONFIG_PATH` or ~/.openclaw/openclaw.json).
    pub openclaw_config_path: Option<PathBuf>,
    /// Plugin path or npm spec for `openclaw plugins install`.
    pub plugin: Option<String>,
    /// Override the seashail binary path stored in plugin config.
    pub seashail_path: Option<PathBuf>,
    /// Boolean flags for the install operation.
    pub flags: InstallFlags,
}

fn seashail_command_for_openclaw(seashail_path_override: Option<&Path>) -> eyre::Result<String> {
    if let Some(p) = seashail_path_override {
        return Ok(p
            .to_str()
            .ok_or_else(|| eyre::eyre!("seashail path must be valid unicode"))?
            .to_owned());
    }

    // Use an absolute path by default.
    //
    // Rationale: OpenClaw's gateway often runs as a user service (launchd/systemd),
    // where PATH may not include ~/.cargo/bin. Using "seashail" would make the
    // plugin fail to spawn the binary even though it works in an interactive shell.
    Ok(std::env::current_exe()
        .context("resolve current exe")?
        .to_string_lossy()
        .to_string())
}

fn default_openclaw_config_path() -> eyre::Result<PathBuf> {
    if let Ok(p) = std::env::var("OPENCLAW_CONFIG_PATH") {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let home = directories::UserDirs::new()
        .context("resolve home dir")?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".openclaw/openclaw.json"))
}

fn repo_local_plugin_path_if_present() -> Option<PathBuf> {
    // For local development (this repo), prefer the in-tree plugin folder.
    let cwd = std::env::current_dir().ok()?;
    let p = cwd.join("packages/openclaw-seashail-plugin");
    p.is_dir().then_some(p)
}

fn is_probably_local_path(s: &str) -> bool {
    let p = Path::new(s);
    p.is_absolute() || s.starts_with("./") || s.starts_with("../")
}

fn read_json_file(path: &Path) -> eyre::Result<Value> {
    let s = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let v: Value = serde_json::from_str(&s).with_context(|| format!("parse {}", path.display()))?;
    Ok(v)
}

fn write_json_file_atomic(path: &Path, v: &Value) -> eyre::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| eyre::eyre!("openclaw config path must have a parent dir"))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;

    let perms = fs::metadata(path).ok().map(|m| m.permissions());

    let s = serde_json::to_string_pretty(v).context("serialize json")?;
    let tmp = parent.join(format!(
        ".{}.seashail.tmp",
        path.file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("openclaw.json")
    ));
    fs::write(&tmp, format!("{s}\n")).with_context(|| format!("write {}", tmp.display()))?;

    if let Some(perms) = perms {
        if let Err(_e) = fs::set_permissions(&tmp, perms) {
            // Best-effort: ignore permission copy errors.
        }
    }

    // Best-effort backup next to config (do not overwrite existing).
    let bak = parent.join(format!(
        "{}.bak.seashail",
        path.file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("openclaw.json")
    ));
    if path.exists() && !bak.exists() {
        if let Err(_e) = fs::copy(path, &bak) {
            // Best-effort: ignore backup failures.
        }
    }

    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

fn ensure_object(v: &mut Value) -> eyre::Result<&mut serde_json::Map<String, Value>> {
    if !v.is_object() {
        *v = Value::Object(serde_json::Map::new());
    }
    v.as_object_mut().context("expected json object")
}

fn get_or_insert_object<'a>(
    parent: &'a mut serde_json::Map<String, Value>,
    key: &str,
) -> eyre::Result<&'a mut serde_json::Map<String, Value>> {
    let entry = parent.entry(key.to_owned()).or_insert_with(|| json!({}));
    ensure_object(entry)
}

fn patch_openclaw_config(
    root: &mut Value,
    network: AgentNetwork,
    seashail_cmd: &str,
    enable_in_sandbox: bool,
    plugin_load_path: Option<&str>,
) -> eyre::Result<()> {
    let obj = ensure_object(root)?;
    let plugins = get_or_insert_object(obj, "plugins")?;
    if let Some(p) = plugin_load_path {
        // OpenClaw discovers non-bundled plugins from `plugins.load.paths`.
        // Ensure our plugin path is present so `plugins.entries.seashail` validates.
        let load = get_or_insert_object(plugins, "load")?;
        let paths = load
            .entry("paths".to_owned())
            .or_insert_with(|| Value::Array(vec![]));
        if !paths.is_array() {
            *paths = Value::Array(vec![]);
        }
        let arr = paths
            .as_array_mut()
            .context("plugins.load.paths must be an array")?;
        let already = arr.iter().any(|v| v.as_str() == Some(p));
        if !already {
            arr.push(Value::String(p.to_owned()));
        }
    }
    let entries = get_or_insert_object(plugins, "entries")?;
    let seashail = entries
        .entry("seashail".to_owned())
        .or_insert_with(|| json!({}));
    let seashail_obj = ensure_object(seashail)?;

    seashail_obj.insert("enabled".to_owned(), Value::Bool(true));

    let cfg = seashail_obj
        .entry("config".to_owned())
        .or_insert_with(|| json!({}));
    let cfg_obj = ensure_object(cfg)?;

    cfg_obj.insert(
        "seashailPath".to_owned(),
        Value::String(seashail_cmd.to_owned()),
    );
    cfg_obj.insert(
        "network".to_owned(),
        Value::String(match network {
            AgentNetwork::Mainnet => "mainnet".to_owned(),
            AgentNetwork::Testnet => "testnet".to_owned(),
        }),
    );

    // Default to prefixing tool names to avoid collisions with other plugins/tools.
    cfg_obj.insert("toolPrefix".to_owned(), Value::Bool(true));
    cfg_obj.insert("prefix".to_owned(), Value::String("seashail_".to_owned()));
    cfg_obj.insert(
        "passphraseEnvVar".to_owned(),
        Value::String("SEASHAIL_PASSPHRASE".to_owned()),
    );
    cfg_obj.insert("standalone".to_owned(), Value::Bool(false));

    // Ensure plugin tools are visible to agents by default.
    //
    // OpenClaw tool policy supports either:
    // - `tools.allow` (authoritative allowlist), or
    // - `tools.profile` + `tools.alsoAllow` (additive plugin enablement).
    //
    // We prefer `alsoAllow` unless the user already has an explicit `allow` list.
    {
        let tools = get_or_insert_object(obj, "tools")?;
        let allow_entry = tools.get_mut("allow");
        let allow_is_present = allow_entry.is_some();
        if allow_is_present {
            // If allow exists, keep policy shape and just ensure "seashail" is included.
            let allow = tools
                .entry("allow".to_owned())
                .or_insert_with(|| Value::Array(vec![]));
            if !allow.is_array() {
                *allow = Value::Array(vec![]);
            }
            let arr = allow
                .as_array_mut()
                .context("tools.allow must be an array")?;
            let already = arr.iter().any(|v| v.as_str() == Some("seashail"));
            if !already {
                arr.push(Value::String("seashail".to_owned()));
            }
        } else {
            let also = tools
                .entry("alsoAllow".to_owned())
                .or_insert_with(|| Value::Array(vec![]));
            if !also.is_array() {
                *also = Value::Array(vec![]);
            }
            let arr = also
                .as_array_mut()
                .context("tools.alsoAllow must be an array")?;
            let already = arr.iter().any(|v| v.as_str() == Some("seashail"));
            if !already {
                arr.push(Value::String("seashail".to_owned()));
            }
        }
    }

    if enable_in_sandbox {
        // OpenClaw uses a sandbox allowlist for tools. We add our plugin id so sandboxed agents
        // can invoke it when configured to run in sandbox mode.
        let tools = get_or_insert_object(obj, "tools")?;
        let sandbox = get_or_insert_object(tools, "sandbox")?;
        let sandbox_tools = get_or_insert_object(sandbox, "tools")?;
        let allow = sandbox_tools
            .entry("allow".to_owned())
            .or_insert_with(|| Value::Array(vec![]));
        if !allow.is_array() {
            *allow = Value::Array(vec![]);
        }
        let arr = allow.as_array_mut().context("allow must be an array")?;
        let already = arr.iter().any(|v| v.as_str() == Some("seashail"));
        if !already {
            arr.push(Value::String("seashail".to_owned()));
        }
    }

    Ok(())
}

fn run_openclaw(args: &[&str]) -> eyre::Result<()> {
    let st = Command::new("openclaw")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .with_context(|| format!("run `openclaw {}`", args.join(" ")))?;
    if !st.success() {
        eyre::bail!("openclaw command failed: `openclaw {}`", args.join(" "));
    }
    Ok(())
}

fn run_openclaw_best_effort(args: &[&str]) {
    // Intentionally ignore failures. Some OpenClaw subcommands are not available in all
    // installations (e.g. gateway service mgmt) and we don't want installs to fail
    // purely because the user isn't running the gateway as a service.
    let _status = Command::new("openclaw")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub fn install(opts: InstallOpts) -> eyre::Result<()> {
    // Ensure OpenClaw exists on PATH early.
    run_openclaw(&["--version"]).context("openclaw not found (install OpenClaw first)")?;

    let config_path = match opts.openclaw_config_path {
        Some(p) => p,
        None => default_openclaw_config_path()?,
    };

    let (plugin, link_default) = match opts.plugin {
        Some(p) => (p, false),
        None => {
            if let Some(local) = repo_local_plugin_path_if_present() {
                (local.to_string_lossy().to_string(), true)
            } else {
                ("@seashail/seashail".to_owned(), false)
            }
        }
    };

    let link = opts.flags.link.get() || link_default;
    let plugin_load_path: Option<String> = if is_probably_local_path(&plugin) {
        // Canonicalize when possible to keep OpenClaw path matching stable.
        let pb = PathBuf::from(&plugin);
        match std::fs::canonicalize(&pb) {
            Ok(abs) => abs.to_str().map(std::borrow::ToOwned::to_owned),
            Err(_) => pb.to_str().map(std::borrow::ToOwned::to_owned),
        }
    } else {
        None
    };

    // 1) Install plugin (path or npm spec).
    if link && !is_probably_local_path(&plugin) {
        eyre::bail!("--link is only valid when installing from a local path");
    }

    // OpenClaw's `plugins install` is not idempotent: it exits non-zero if the plugin already exists.
    // Make `seashail openclaw install` safe to re-run by skipping the install step when the plugin
    // directory already exists.
    let extensions_dir = config_path
        .parent()
        .ok_or_else(|| eyre::eyre!("openclaw config path must have a parent dir"))?
        .join("extensions");
    let plugin_dir = extensions_dir.join("seashail");

    let did_install = if plugin_dir.exists() {
        false
    } else {
        if link {
            run_openclaw(&["plugins", "install", "-l", &plugin])
                .context("install openclaw plugin (link)")?;
        } else {
            run_openclaw(&["plugins", "install", &plugin]).context("install openclaw plugin")?;
        }
        true
    };

    // 2) Patch openclaw.json to enable + configure plugin.
    let seashail_cmd = seashail_command_for_openclaw(opts.seashail_path.as_deref())?;
    let mut root = if config_path.exists() {
        read_json_file(&config_path)?
    } else {
        json!({})
    };
    patch_openclaw_config(
        &mut root,
        opts.network,
        &seashail_cmd,
        opts.flags.enable_in_sandbox.get(),
        plugin_load_path.as_deref(),
    )?;
    write_json_file_atomic(&config_path, &root)?;

    // 3) Ensure plugin enabled (OpenClaw CLI handles any extra wiring).
    if let Err(_e) = run_openclaw(&["plugins", "enable", "seashail"]) {
        // Best-effort; if this fails, OpenClaw may already have the plugin enabled.
    }

    // 4) Restart gateway to pick up config changes.
    if opts.flags.restart_gateway.get() {
        // Ensure the gateway is actually running. On macOS/Linux/Windows, OpenClaw supports a
        // user-level gateway service (launchd/systemd/schtasks). If the user hasn't installed it,
        // `openclaw tui` will fail with "gateway not connected".
        //
        // These are best-effort: some users may prefer running `openclaw gateway run` manually.
        run_openclaw_best_effort(&["gateway", "install"]);
        run_openclaw_best_effort(&["gateway", "start"]);
        run_openclaw_best_effort(&["gateway", "restart"]);
    }

    // 5) Seamless onboarding: ensure a default wallet exists so OpenClaw can immediately use Seashail.
    if opts.flags.onboard_wallet.get() {
        ensure_default_wallet_cli().context("ensure default wallet")?;
    }

    // Minimal machine-readable confirmation (do not print openclaw.json contents; it may contain secrets).
    let out = serde_json::to_string(&json!({
      "ok": true,
      "openclaw_config_path": config_path,
      "plugin": plugin,
      "linked": link,
      "installed": did_install,
      "restart_gateway": opts.flags.restart_gateway.get(),
      "network": match opts.network { AgentNetwork::Mainnet => "mainnet", AgentNetwork::Testnet => "testnet" },
    }))
    .unwrap_or_else(|_| "{\"ok\":true}".to_owned());
    {
        let mut stdout = std::io::stdout().lock();
        if let Err(_e) = stdout.write_all(out.as_bytes()) {
            // Best-effort; ignore stdout write errors.
        }
        if let Err(_e) = stdout.write_all(b"\n") {
            // Best-effort; ignore stdout write errors.
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_adds_plugin_entry_and_sandbox_allow() {
        let mut root = json!({
          "gateway": { "port": 18_789_i64 },
          "plugins": { "entries": { "slack": { "enabled": true } } }
        });
        assert!(patch_openclaw_config(
            &mut root,
            AgentNetwork::Testnet,
            "/tmp/seashail",
            true,
            None
        )
        .is_ok());

        let p = root
            .pointer("/plugins/entries/seashail/enabled")
            .and_then(Value::as_bool);
        assert_eq!(p, Some(true));

        let net = root
            .pointer("/plugins/entries/seashail/config/network")
            .and_then(Value::as_str);
        assert_eq!(net, Some("testnet"));

        let allow_has = root
            .pointer("/tools/sandbox/tools/allow")
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("seashail")));
        assert!(allow_has);
    }
}

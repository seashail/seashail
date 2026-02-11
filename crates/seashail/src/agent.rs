use eyre::{Context as _, ContextCompat as _};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTarget {
    Cursor,
    VsCode,
    Windsurf,
    ClaudeDesktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentNetwork {
    Mainnet,
    Testnet,
}

fn path_has_executable(name: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&paths) {
        let p = dir.join(name);
        if p.is_file() {
            return true;
        }
        // Windows: allow `seashail.exe` even if config uses `seashail`.
        #[cfg(windows)]
        {
            let p = dir.join(format!("{name}.exe"));
            if p.is_file() {
                return true;
            }
        }
    }
    false
}

fn seashail_command_for_templates() -> String {
    // Prefer a stable PATH-based command when available so upgrades don't stale configs.
    // Fall back to an absolute path for dev builds (`./target/debug/seashail agent install ...`)
    // where `seashail` likely isn't on PATH yet.
    if path_has_executable("seashail") {
        return "seashail".to_owned();
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_else(|| "seashail".to_owned())
}

fn seashail_args(network: AgentNetwork) -> Vec<String> {
    match network {
        AgentNetwork::Mainnet => vec!["mcp".into()],
        AgentNetwork::Testnet => vec!["mcp".into(), "--network".into(), "testnet".into()],
    }
}

fn cursor_template(network: AgentNetwork) -> Value {
    let cmd = seashail_command_for_templates();
    json!({
      "mcpServers": {
        "seashail": {
          "command": cmd,
          "args": seashail_args(network),
        }
      }
    })
}

fn windsurf_template(network: AgentNetwork) -> Value {
    // Windsurf uses an MCP config file with the same `mcpServers` shape.
    cursor_template(network)
}

fn vscode_template(network: AgentNetwork) -> Value {
    let cmd = seashail_command_for_templates();
    json!({
      "servers": {
        "seashail": {
          "type": "stdio",
          "command": cmd,
          "args": seashail_args(network),
        }
      }
    })
}

pub fn print_template(target: AgentTarget, network: AgentNetwork) -> eyre::Result<()> {
    let v = match target {
        AgentTarget::Cursor | AgentTarget::ClaudeDesktop => cursor_template(network),
        AgentTarget::VsCode => vscode_template(network),
        AgentTarget::Windsurf => windsurf_template(network),
    };
    let mut out = io::stdout().lock();
    writeln!(
        &mut out,
        "{}",
        serde_json::to_string_pretty(&v).context("serialize template")?
    )
    .context("write template to stdout")?;
    Ok(())
}

fn default_install_path(target: AgentTarget) -> eyre::Result<PathBuf> {
    match target {
        AgentTarget::Cursor => Ok(PathBuf::from(".cursor/mcp.json")),
        AgentTarget::VsCode => Ok(PathBuf::from(".vscode/mcp.json")),
        AgentTarget::Windsurf => {
            let home = directories::UserDirs::new()
                .context("resolve home dir")?
                .home_dir()
                .to_path_buf();
            Ok(home.join(".codeium/windsurf/mcp_config.json"))
        }
        AgentTarget::ClaudeDesktop => claude_desktop_default_path(),
    }
}

fn claude_desktop_default_path() -> eyre::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = directories::UserDirs::new()
            .context("resolve home dir")?
            .home_dir()
            .to_path_buf();
        Ok(home.join("Library/Application Support/Claude/claude_desktop_config.json"))
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").context("resolve %APPDATA%")?;
        Ok(PathBuf::from(appdata).join("Claude/claude_desktop_config.json"))
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let home = directories::UserDirs::new()
            .context("resolve home dir")?
            .home_dir()
            .to_path_buf();
        Ok(home.join(".config/Claude/claude_desktop_config.json"))
    }
}

fn insert_server(
    root: &mut Value,
    servers_key: &str,
    server_name: &str,
    server: Value,
) -> eyre::Result<()> {
    if !root.is_object() {
        *root = json!({});
    }
    let Some(obj) = root.as_object_mut() else {
        eyre::bail!("root must be an object");
    };
    let servers = obj.entry(servers_key).or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    let Some(s) = servers.as_object_mut() else {
        eyre::bail!("{servers_key} must be an object");
    };
    s.insert(server_name.to_owned(), server);
    Ok(())
}

fn extract_server_entry(template: &Value, servers_key: &str) -> eyre::Result<Value> {
    let obj = template
        .as_object()
        .ok_or_else(|| eyre::eyre!("template must be an object"))?;
    let servers = obj
        .get(servers_key)
        .ok_or_else(|| eyre::eyre!("template missing {servers_key}"))?;
    let servers_obj = servers
        .as_object()
        .ok_or_else(|| eyre::eyre!("{servers_key} must be an object"))?;
    let seashail = servers_obj
        .get("seashail")
        .ok_or_else(|| eyre::eyre!("template missing {servers_key}.seashail"))?;
    Ok(seashail.clone())
}

fn read_json_file(path: &Path) -> eyre::Result<Value> {
    let s = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let v: Value = serde_json::from_str(&s).with_context(|| format!("parse {}", path.display()))?;
    Ok(v)
}

fn write_json_file(path: &Path, v: &Value) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let s = serde_json::to_string_pretty(v).context("serialize json")?;
    fs::write(path, format!("{s}\n")).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn install_template(
    target: AgentTarget,
    network: AgentNetwork,
    path: Option<PathBuf>,
) -> eyre::Result<()> {
    let path = match path {
        Some(p) => p,
        None => default_install_path(target)?,
    };

    let (template, servers_key) = match target {
        AgentTarget::Cursor | AgentTarget::ClaudeDesktop => {
            (cursor_template(network), "mcpServers")
        }
        AgentTarget::Windsurf => (windsurf_template(network), "mcpServers"),
        AgentTarget::VsCode => (vscode_template(network), "servers"),
    };

    let server = extract_server_entry(&template, servers_key)?;

    let mut root = if path.exists() {
        read_json_file(&path)?
    } else {
        json!({})
    };

    insert_server(&mut root, servers_key, "seashail", server)?;
    write_json_file(&path, &root)?;

    // Print a minimal confirmation line (human-readable).
    let mut out = io::stdout().lock();
    writeln!(&mut out, "{}", json!({ "ok": true, "path": path }))
        .context("write install confirmation")?;
    Ok(())
}

pub fn supported_agents() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("cursor", "Cursor (.cursor/mcp.json)"),
        ("vscode", "VS Code / GitHub Copilot (.vscode/mcp.json)"),
        ("windsurf", "Windsurf (~/.codeium/windsurf/mcp_config.json)"),
        (
            "claude-desktop",
            "Claude Desktop (recommended; OS-specific claude_desktop_config.json)",
        ),
    ])
}

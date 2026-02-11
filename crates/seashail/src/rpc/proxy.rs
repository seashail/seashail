use crate::config::NetworkMode;
use crate::paths::SeashailPaths;
use eyre::Context as _;
use serde_json::{Map, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

#[cfg(unix)]
use tokio::net::UnixStream;

#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

#[cfg(not(any(unix, windows)))]
use tokio::net::TcpStream;

#[cfg(unix)]
fn socket_path(paths: &SeashailPaths) -> std::path::PathBuf {
    paths.data_dir.join("seashail-mcp.sock")
}

#[cfg(windows)]
fn pipe_name(paths: &SeashailPaths) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    paths.data_dir.to_string_lossy().hash(&mut h);
    format!(r"\\.\pipe\seashail-mcp-{:016x}", h.finish())
}

fn spawn_daemon() -> eyre::Result<()> {
    let exe = std::env::current_exe().context("resolve current exe")?;
    let idle = std::env::var("SEASHAIL_DAEMON_IDLE_EXIT_SECONDS").unwrap_or_else(|_| "60".into());

    // Best-effort background daemon. We keep it simple: inherit env, detach stdio, and rely
    // on idle-exit so tests/agents don't leave stragglers.
    let _child = std::process::Command::new(exe)
        .arg("daemon")
        .arg("--idle-exit-seconds")
        .arg(idle)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawn seashail daemon")?;

    Ok(())
}

async fn try_connect(paths: &SeashailPaths) -> eyre::Result<ProxyConn> {
    #[cfg(unix)]
    {
        let p = socket_path(paths);
        let s = UnixStream::connect(&p)
            .await
            .with_context(|| format!("connect unix socket at {}", p.display()))?;
        Ok(ProxyConn::Unix(s))
    }
    #[cfg(windows)]
    {
        let name = pipe_name(paths);
        let s = ClientOptions::new()
            .open(&name)
            .with_context(|| format!("connect named pipe at {name}"))?;
        Ok(ProxyConn::Pipe(s))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let s = TcpStream::connect("127.0.0.1:41777")
            .await
            .context("connect tcp daemon")?;
        Ok(ProxyConn::Tcp(s))
    }
}

enum ProxyConn {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Pipe(NamedPipeClient),
    #[cfg(not(any(unix, windows)))]
    Tcp(TcpStream),
}

fn inject_params(line: &str, network: Option<NetworkMode>, auth: &str) -> String {
    let Ok(mut v) = serde_json::from_str::<Value>(line) else {
        return line.to_owned();
    };

    let method = v
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_owned();
    if method.is_empty() {
        return line.to_owned();
    }

    let Some(obj) = v.as_object_mut() else {
        return line.to_owned();
    };

    let params = obj
        .entry("params")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(pobj) = params.as_object_mut() {
        pobj.insert("seashail_auth".to_owned(), Value::String(auth.to_owned()));

        // Optional: session-only network override.
        if method == "initialize" {
            if let Some(mode) = network {
                pobj.insert(
                    "seashail_network_override".to_owned(),
                    Value::String(match mode {
                        NetworkMode::Mainnet => "mainnet".to_owned(),
                        NetworkMode::Testnet => "testnet".to_owned(),
                    }),
                );
            }
        }
    }

    serde_json::to_string(&v).unwrap_or_else(|_| line.to_owned())
}

/// Run an MCP stdio proxy that forwards to a singleton local daemon.
pub async fn run(network_override: Option<NetworkMode>) -> eyre::Result<()> {
    let paths = SeashailPaths::discover()?;
    let auth = paths.ensure_auth_token()?;

    let mut conn = try_connect(&paths).await;
    if conn.is_err() {
        spawn_daemon()?;
        for _ in 0_i32..50_i32 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Ok(c) = try_connect(&paths).await {
                conn = Ok(c);
                break;
            }
        }
    }
    let conn = conn.context("connect to seashail daemon")?;

    let (r, mut w) = match conn {
        #[cfg(unix)]
        ProxyConn::Unix(s) => tokio::io::split(s),
        #[cfg(windows)]
        ProxyConn::Pipe(s) => tokio::io::split(s),
        #[cfg(not(any(unix, windows)))]
        ProxyConn::Tcp(s) => tokio::io::split(s),
    };

    let mut daemon_lines = BufReader::new(r).lines();
    let mut agent_lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    // Prime auth on the daemon connection.
    //
    // The daemon enforces `seashail_auth` only on the first request of a connection.
    // By sending an authenticated ping up-front, we make the proxy robust against any
    // client messages that might arrive before injection (or malformed lines that the
    // proxy chooses not to rewrite).
    {
        let ping = inject_params(
            r#"{"jsonrpc":"2.0","id":0,"method":"ping","params":{}}"#,
            None,
            &auth,
        );
        w.write_all(ping.as_bytes()).await?;
        w.write_all(b"\n").await?;
        w.flush().await?;
        // Best-effort: drain the ping response so it doesn't get forwarded to the client.
        drop(tokio::time::timeout(Duration::from_millis(500), daemon_lines.next_line()).await);
    }

    // Forward agent->daemon and daemon->agent concurrently.
    let agent_to_daemon = async {
        while let Some(line) = agent_lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            if line.len() > crate::rpc::server::MAX_JSONRPC_LINE_BYTES {
                eyre::bail!("jsonrpc line too large");
            }
            let out = inject_params(&line, network_override, &auth);
            w.write_all(out.as_bytes()).await?;
            w.write_all(b"\n").await?;
            w.flush().await?;
        }
        Ok::<(), eyre::Report>(())
    };

    let daemon_to_agent = async {
        while let Some(line) = daemon_lines.next_line().await? {
            stdout.write_all(line.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
        Ok::<(), eyre::Report>(())
    };

    tokio::select! {
        agent_res = agent_to_daemon => agent_res?,
        daemon_res = daemon_to_agent => daemon_res?,
    }

    Ok(())
}

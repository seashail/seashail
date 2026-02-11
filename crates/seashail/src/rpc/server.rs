use crate::{
    config::NetworkMode,
    keystore::Keystore,
    paths::SeashailPaths,
    rpc::mcp_server::{self, ConnState, JsonRpcResponse, SharedState},
};
use eyre::Context as _;
use fs2::FileExt as _;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tracing::warn;

pub const MAX_JSONRPC_LINE_BYTES: usize = 1_000_000;

struct ActiveGuard {
    active: Arc<AtomicUsize>,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(unix)]
use tokio::net::UnixListener;

#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

#[cfg(not(any(unix, windows)))]
use tokio::net::TcpListener;

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcNotification {
    jsonrpc: String,
}

async fn write_frame<W, T>(out: &mut W, v: &T) -> eyre::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin + Send,
    T: serde::Serialize + Sync,
{
    out.write_all(format!("{}\n", serde_json::to_string(v)?).as_bytes())
        .await?;
    out.flush().await?;
    Ok(())
}

fn parse_network_override(params: &Value) -> Option<NetworkMode> {
    params
        .get("seashail_network_override")
        .and_then(|v| v.as_str())
        .and_then(|s| match s.trim().to_lowercase().as_str() {
            "mainnet" | "main" | "prod" | "production" => Some(NetworkMode::Mainnet),
            "testnet" | "test" | "dev" | "devnet" => Some(NetworkMode::Testnet),
            _ => None,
        })
}

#[cfg(unix)]
fn bind_listener(paths: &SeashailPaths) -> eyre::Result<UnixListener> {
    let p = paths.data_dir.join("seashail-mcp.sock");
    if p.exists() {
        let md = std::fs::symlink_metadata(&p).context("stat existing socket path")?;
        if md.file_type().is_symlink() {
            eyre::bail!("refusing to remove symlink at {}", p.display());
        }
        std::fs::remove_file(&p)
            .with_context(|| format!("remove existing socket at {}", p.display()))?;
    }
    if let Some(parent) = p.parent() {
        crate::fsutil::ensure_private_dir(parent)?;
    }
    let l =
        UnixListener::bind(&p).with_context(|| format!("bind unix socket at {}", p.display()))?;

    // Best-effort: lock down the socket file perms.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if let Err(e) = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)) {
            warn!(error = %e, "failed to set unix socket permissions");
        }
    }

    Ok(l)
}

#[cfg(windows)]
fn pipe_name(paths: &SeashailPaths) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    paths.data_dir.to_string_lossy().hash(&mut h);
    format!(r"\\.\pipe\seashail-mcp-{:016x}", h.finish())
}

#[cfg(windows)]
fn create_pipe(paths: &SeashailPaths, first: bool) -> eyre::Result<NamedPipeServer> {
    let name = pipe_name(paths);
    let mut opts = ServerOptions::new();
    if first {
        opts = opts.first_pipe_instance(true);
    }
    opts.create(&name)
        .with_context(|| format!("create named pipe server at {name}"))
}

#[cfg(not(any(unix, windows)))]
async fn bind_listener(_paths: &SeashailPaths) -> eyre::Result<TcpListener> {
    TcpListener::bind("127.0.0.1:41777")
        .await
        .context("bind tcp listener (loopback)")
}

/// Dispatch a parsed JSON-RPC request to the appropriate handler.
async fn dispatch_request<R, W>(
    req: JsonRpcRequest,
    shared: &Arc<tokio::sync::Mutex<SharedState>>,
    conn: &mut ConnState,
    lines: &mut tokio::io::Lines<tokio::io::BufReader<R>>,
    w: &mut W,
) -> eyre::Result<JsonRpcResponse>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    let resp = match req.method.as_str() {
        "initialize" => {
            if let Some(m) = parse_network_override(&req.params) {
                conn.network_override = Some(m);
            }
            // Eagerly ensure the generated `default` wallet exists for seamless onboarding.
            // This runs in the daemon (proxy mode) so OpenClaw can show deposit addresses
            // immediately after startup, before any wallet-dependent tool call.
            {
                let guard = shared.lock().await;
                if let Err(e) = guard.ks.ensure_default_wallet() {
                    warn!(error = %e, "ensure default wallet failed during initialize");
                }
            }
            mcp_server::ok(
                req.id,
                json!({
                  "protocolVersion": "2025-06-18",
                  "serverInfo": { "name": "seashail", "version": env!("CARGO_PKG_VERSION") },
                  "capabilities": { "tools": {}, "elicitation": { "form": {} } }
                }),
            )
        }
        "ping" => mcp_server::ok(req.id, json!({})),
        "tools/list" => mcp_server::ok(req.id, mcp_server::list_tools_result()),
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|name_v| name_v.as_str())
                .unwrap_or("");
            let args = req.params.get("arguments").cloned().unwrap_or(Value::Null);
            let id = req.id.clone();

            let mut guard = shared.lock().await;
            match mcp_server::handle_tools_call(id.clone(), name, args, &mut guard, conn, lines, w)
                .await
            {
                Ok(tool_resp) => tool_resp,
                Err(e) => {
                    if let Some(se) = e.downcast_ref::<crate::errors::SeashailError>() {
                        mcp_server::ok(
                            id,
                            mcp_server::tool_err(crate::errors::ToolError::from(se.clone())),
                        )
                    } else {
                        let te = crate::errors::ToolError::new("internal_error", format!("{e:#}"));
                        mcp_server::ok(id, mcp_server::tool_err(te))
                    }
                }
            }
        }
        _ => mcp_server::err(req.id, -32601, "method not found"),
    };
    Ok(resp)
}

/// Parse and validate a JSON-RPC line. Returns `None` if the line should be skipped.
fn parse_jsonrpc_line(line: &str) -> Option<Result<JsonRpcRequest, (Value, i32, &'static str)>> {
    let req_v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "invalid json from client");
            return None;
        }
    };

    if req_v.get("id").is_none() {
        if let Ok(note) = serde_json::from_value::<JsonRpcNotification>(req_v.clone()) {
            if note.jsonrpc == "2.0" {
                return None;
            }
        }
    }

    let req: JsonRpcRequest = match serde_json::from_value(req_v) {
        Ok(parsed_req) => parsed_req,
        Err(e) => {
            warn!(error = %e, "failed to parse jsonrpc request");
            return None;
        }
    };

    if req.jsonrpc != "2.0" {
        return Some(Err((req.id, -32600, "invalid jsonrpc version")));
    }

    Some(Ok(req))
}

async fn serve_connection<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + Sync>(
    stream: S,
    shared: Arc<tokio::sync::Mutex<SharedState>>,
    active: Arc<AtomicUsize>,
    auth_token: Arc<str>,
) -> eyre::Result<()> {
    active.fetch_add(1, Ordering::SeqCst);
    let _guard = ActiveGuard {
        active: Arc::clone(&active),
    };

    let (read_half, mut w) = tokio::io::split(stream);
    let mut lines = BufReader::new(read_half).lines();
    let mut conn = ConnState::new();
    let mut authed = false;

    while let Some(line) = lines.next_line().await? {
        if line.len() > MAX_JSONRPC_LINE_BYTES {
            break;
        }

        let req = match parse_jsonrpc_line(&line) {
            Some(Ok(parsed)) => parsed,
            Some(Err((id, code, msg))) => {
                write_frame(&mut w, &mcp_server::err(id, i64::from(code), msg)).await?;
                continue;
            }
            None => continue,
        };

        if !authed {
            let got = req
                .params
                .get("seashail_auth")
                .and_then(|auth_v| auth_v.as_str())
                .unwrap_or("");
            if got != auth_token.as_ref() {
                if let Err(e) =
                    write_frame(&mut w, &mcp_server::err(req.id, -32001, "unauthorized")).await
                {
                    warn!(error = %e, "failed to write unauthorized response");
                }
                break;
            }
            authed = true;
        }

        let resp = dispatch_request(req, &shared, &mut conn, &mut lines, &mut w).await?;
        write_frame(&mut w, &resp).await?;
    }

    Ok(())
}

pub async fn run_daemon(idle_exit_seconds: Option<u64>) -> eyre::Result<()> {
    let paths = SeashailPaths::discover()?;
    paths.ensure_private_dirs()?;
    let auth: Arc<str> = Arc::from(paths.ensure_auth_token()?);

    // Single-instance lock: ensures only one daemon owns the keystore + passphrase session.
    crate::fsutil::ensure_private_dir(&paths.data_dir)?;
    let lock_path = paths.data_dir.join("seashail-daemon.lock");
    let lock_file = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .mode(0o600)
                .open(&lock_path)
                .with_context(|| format!("open lock file at {}", lock_path.display()))?
        }
        #[cfg(not(unix))]
        {
            std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&lock_path)
                .with_context(|| format!("open lock file at {}", lock_path.display()))?
        }
    };
    lock_file
        .try_lock_exclusive()
        .with_context(|| format!("lock already held at {}", lock_path.display()))?;
    // Keep the lock held for daemon lifetime.
    let _lock_file = lock_file;

    let ks = Keystore::open(paths.clone())?;
    // Daemon should default to a stable shared DB file so cache persists across restarts.
    let shared = Arc::new(tokio::sync::Mutex::new(SharedState::new(ks, true)?));
    let active = Arc::new(AtomicUsize::new(0));

    #[cfg(unix)]
    let listener = bind_listener(&paths)?;
    #[cfg(not(any(unix, windows)))]
    let listener = bind_listener(&paths).await?;
    #[cfg(windows)]
    let mut next_pipe = create_pipe(&paths, true)?;

    let idle = idle_exit_seconds.map(Duration::from_secs);
    let mut last_empty = Instant::now();

    loop {
        if active.load(Ordering::SeqCst) == 0 {
            if let Some(idle_dur) = idle {
                if last_empty.elapsed() >= idle_dur {
                    break;
                }
            }
        } else {
            last_empty = Instant::now();
        }

        #[cfg(any(unix, not(any(unix, windows))))]
        let stream = {
            let accept_fut = async {
                let (stream, _addr) = listener.accept().await?;
                Ok::<_, eyre::Report>(stream)
            };
            match idle {
                Some(_) => match tokio::time::timeout(Duration::from_millis(250), accept_fut).await
                {
                    Ok(res) => res?,
                    Err(_) => continue,
                },
                None => accept_fut.await?,
            }
        };

        #[cfg(windows)]
        let stream = {
            // Prepare the next instance before awaiting connect, so multiple clients can connect.
            let current = next_pipe;
            next_pipe = create_pipe(&paths, false)?;

            let connect_fut = async {
                current.connect().await?;
                Ok::<_, eyre::Report>(current)
            };

            match idle {
                Some(_) => {
                    match tokio::time::timeout(Duration::from_millis(250), connect_fut).await {
                        Ok(res) => res?,
                        Err(_) => continue,
                    }
                }
                None => connect_fut.await?,
            }
        };

        let shared2 = Arc::clone(&shared);
        let active2 = Arc::clone(&active);
        let auth2 = Arc::clone(&auth);
        tokio::spawn(async move {
            if let Err(e) = serve_connection(stream, shared2, active2, auth2).await {
                warn!(error = %e, "connection handler failed");
            }
        });
    }

    Ok(())
}

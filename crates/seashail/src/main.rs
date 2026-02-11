#![recursion_limit = "256"]
#![expect(
    clippy::multiple_crate_versions,
    reason = "transitive dependency duplication"
)]

use clap::{Parser, Subcommand, ValueEnum};
use eyre::Context as _;
use std::io::IsTerminal as _;
use tracing_subscriber::prelude::*;

mod agent;
mod amount;
mod audit;
mod blocklist;
mod chains;
mod cli_output;
mod config;
mod db;
mod doctor;
mod errors;
mod financial_math;
mod fsutil;
mod keystore;
mod marketplace_adapter;
mod ofac;
mod openclaw;
mod paths;
mod perps;
mod policy;
mod policy_engine;
mod price;
mod retry;
mod rpc;
mod store;
mod upgrade;
mod wallet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliNetworkMode {
    Mainnet,
    Testnet,
}

impl From<CliNetworkMode> for crate::agent::AgentNetwork {
    fn from(v: CliNetworkMode) -> Self {
        match v {
            CliNetworkMode::Mainnet => Self::Mainnet,
            CliNetworkMode::Testnet => Self::Testnet,
        }
    }
}

impl From<CliNetworkMode> for crate::config::NetworkMode {
    fn from(v: CliNetworkMode) -> Self {
        match v {
            CliNetworkMode::Mainnet => Self::Mainnet,
            CliNetworkMode::Testnet => Self::Testnet,
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "seashail", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliAgentTarget {
    Cursor,
    Vscode,
    Windsurf,
    ClaudeDesktop,
}

impl From<CliAgentTarget> for crate::agent::AgentTarget {
    fn from(v: CliAgentTarget) -> Self {
        match v {
            CliAgentTarget::Cursor => Self::Cursor,
            CliAgentTarget::Vscode => Self::VsCode,
            CliAgentTarget::Windsurf => Self::Windsurf,
            CliAgentTarget::ClaudeDesktop => Self::ClaudeDesktop,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the MCP server over stdio.
    ///
    /// By default this starts a lightweight stdio proxy that forwards to a singleton local
    /// Seashail daemon (so multiple agents share one key/policy state).
    Mcp {
        /// Override config network mode for this session.
        ///
        /// This affects default chain selection when tools omit `chain`/`chains`.
        #[arg(long, value_enum)]
        network: Option<CliNetworkMode>,

        /// Run a standalone MCP server in this process (no daemon, no sharing).
        #[arg(long, default_value_t = false)]
        standalone: bool,
    },

    /// Run the singleton Seashail daemon (shared state across multiple MCP clients).
    Daemon {
        /// Exit after being idle (no active clients) for this many seconds.
        ///
        /// If unset, runs until terminated.
        #[arg(long)]
        idle_exit_seconds: Option<u64>,
    },

    /// Print resolved paths (useful for debugging).
    Paths,

    /// Print a quick self-diagnostic report (safe to paste; contains no secrets).
    Doctor {
        /// Emit JSON to stdout (machine-readable).
        #[arg(long, default_value_t = false)]
        json: bool,
    },

    /// Print or install MCP config templates for popular agents/editors.
    Agent {
        #[command(subcommand)]
        cmd: AgentCommand,
    },

    /// Install or manage the `OpenClaw` plugin integration (first-class `OpenClaw` tools).
    Openclaw {
        #[command(subcommand)]
        cmd: OpenclawCommand,
    },

    /// Upgrade Seashail.
    ///
    /// - If installed via Homebrew on macOS, runs `brew upgrade seashail`.
    /// - Otherwise, re-runs the hosted installer (same mechanism as the npx/uvx wrappers).
    ///
    /// Override auto-detection with `SEASHAIL_UPGRADE_METHOD=brew|installer`.
    Upgrade {
        /// Skip the confirmation prompt (required for non-interactive shells).
        #[arg(long, default_value_t = false)]
        yes: bool,

        /// Suppress installer output (useful for scripts/cron).
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },
}

fn mcp_banner_enabled() -> bool {
    // Default: only show a human banner when stderr is a terminal.
    // Allow forcing on/off via env for debugging.
    match std::env::var("SEASHAIL_BANNER") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !(v.is_empty() || v == "0" || v == "false" || v == "no" || v == "off")
        }
        Err(_) => std::io::stderr().is_terminal(),
    }
}

fn print_mcp_banner(network: Option<CliNetworkMode>, standalone: bool) {
    if !mcp_banner_enabled() {
        return;
    }

    let ver = env!("CARGO_PKG_VERSION");
    let net = match network {
        Some(CliNetworkMode::Mainnet) => "mainnet",
        Some(CliNetworkMode::Testnet) => "testnet",
        None => "auto",
    };
    let mode = if standalone { "standalone" } else { "proxy" };

    // Keep it plain ASCII (portable) and never print secrets/paths here.
    // Banner is intentionally written to stderr for human operators; MCP clients read stdout.
    cli_output::print_mcp_banner(ver, net, mode);
}

#[derive(Subcommand, Debug)]
enum AgentCommand {
    /// List supported agent targets for `print`/`install`.
    List,

    /// Print a full JSON config template to stdout.
    Print {
        #[arg(value_enum)]
        agent: CliAgentTarget,
        /// Optional: emit a testnet template (`seashail mcp --network testnet`).
        #[arg(long, value_enum, default_value_t = CliNetworkMode::Mainnet)]
        network: CliNetworkMode,
    },

    /// Install a config template to a known default location (or `--path`).
    Install {
        #[arg(value_enum)]
        agent: CliAgentTarget,
        /// Optional: install a testnet template (`seashail mcp --network testnet`).
        #[arg(long, value_enum, default_value_t = CliNetworkMode::Mainnet)]
        network: CliNetworkMode,
        /// Override the target config file path.
        #[arg(long)]
        path: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum OpenclawCommand {
    /// Install the Seashail `OpenClaw` plugin and configure `OpenClaw` to enable it.
    ///
    /// This uses the official `OpenClaw` plugin mechanism (no adapter required).
    Install {
        /// Session-only network mode for Seashail when invoked via `OpenClaw`.
        #[arg(long, value_enum, default_value_t = CliNetworkMode::Mainnet)]
        network: CliNetworkMode,

        /// Plugin path or npm spec for `openclaw plugins install`.
        ///
        /// If omitted:
        /// - when run from this repo, uses `./packages/openclaw-seashail-plugin` (linked)
        /// - otherwise uses `@seashail/seashail`
        #[arg(long)]
        plugin: Option<String>,

        /// Link a local plugin path instead of copying (dev-friendly).
        #[arg(long, default_value_t = false)]
        link: bool,

        /// Override the `OpenClaw` config path (defaults to $`OPENCLAW_CONFIG_PATH` or ~/.openclaw/openclaw.json).
        #[arg(long)]
        openclaw_config_path: Option<std::path::PathBuf>,

        /// Override the seashail binary path stored in plugin config (default: "seashail" if on PATH).
        #[arg(long)]
        seashail_path: Option<std::path::PathBuf>,

        /// Restart the `OpenClaw` gateway service after install/config update.
        ///
        /// Use `--restart-gateway false` to skip.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        restart_gateway: bool,

        /// Allow the Seashail plugin in `OpenClaw`'s sandboxed agent mode (tools.sandbox.tools.allow += "seashail").
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        enable_in_sandbox: bool,

        /// After installing/configuring the plugin, ensure Seashail has a default wallet.
        ///
        /// If no wallet exists yet, Seashail will create a machine-local generated wallet named
        /// `default` with EVM/Solana/Bitcoin deposit addresses (no passphrase prompts).
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        onboard_wallet: bool,
    },
}

fn init_logging(paths: &paths::SeashailPaths) -> tracing_appender::non_blocking::WorkerGuard {
    let env_filter = tracing_subscriber::EnvFilter::from_default_env();
    let file_name = paths
        .log_file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("seashail.log.jsonl");
    let file_appender = tracing_appender::rolling::never(&paths.data_dir, file_name);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let stderr_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(std::io::stderr)
        .with_filter(env_filter.clone());
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(file_writer)
        .with_filter(env_filter);

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();

    guard
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    let _: &[policy_engine::WriteOp] = policy_engine::ALL_WRITE_OPS;

    let paths = paths::SeashailPaths::discover()?;
    std::fs::create_dir_all(&paths.data_dir).context("create data dir")?;
    let _log_guard = init_logging(&paths);

    match cli.cmd {
        Command::Mcp {
            network,
            standalone,
        } => {
            upgrade::maybe_auto_upgrade(&paths);
            let net = network.map(Into::into);
            print_mcp_banner(network, standalone);
            if standalone {
                rpc::mcp_server::run(net).await.context("mcp server failed")
            } else {
                rpc::proxy::run(net).await.context("mcp proxy failed")
            }
        }
        Command::Daemon { idle_exit_seconds } => {
            upgrade::maybe_auto_upgrade(&paths);
            rpc::server::run_daemon(idle_exit_seconds)
                .await
                .context("daemon failed")
        }
        Command::Paths => {
            use std::io::Write as _;
            let s = serde_json::to_string(&serde_json::json!({
              "config_dir": paths.config_dir,
              "data_dir": paths.data_dir,
              "log_file": paths.log_file,
            }))
            .context("serialize paths")?;
            writeln!(std::io::stdout().lock(), "{s}").context("write paths")?;
            Ok(())
        }
        Command::Doctor { json } => doctor::run(json).await.context("doctor failed"),
        Command::Agent { cmd } => match cmd {
            AgentCommand::List => {
                use std::io::Write as _;
                let s = serde_json::to_string_pretty(&crate::agent::supported_agents())
                    .context("serialize supported agents")?;
                writeln!(std::io::stdout().lock(), "{s}").context("write supported agents")?;
                Ok(())
            }
            AgentCommand::Print { agent, network } => {
                crate::agent::print_template(agent.into(), network.into())
            }
            AgentCommand::Install {
                agent,
                network,
                path,
            } => crate::agent::install_template(agent.into(), network.into(), path),
        },
        Command::Openclaw { cmd } => match cmd {
            OpenclawCommand::Install {
                network,
                plugin,
                link,
                openclaw_config_path,
                seashail_path,
                restart_gateway,
                enable_in_sandbox,
                onboard_wallet,
            } => crate::openclaw::install(crate::openclaw::InstallOpts {
                network: network.into(),
                openclaw_config_path,
                plugin,
                seashail_path,
                flags: crate::openclaw::InstallFlags {
                    link: link.into(),
                    restart_gateway: restart_gateway.into(),
                    enable_in_sandbox: enable_in_sandbox.into(),
                    onboard_wallet: onboard_wallet.into(),
                },
            })
            .context("openclaw install failed"),
        },
        Command::Upgrade { yes, quiet } => upgrade::run(upgrade::UpgradeOpts { yes, quiet })
            .await
            .context("upgrade failed"),
    }
}

use crate::paths::SeashailPaths;
use chrono::Utc;
use eyre::Context as _;
use fs2::FileExt as _;
use rand::Rng as _;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Stdio,
};

#[derive(Debug, Clone, Copy)]
pub struct UpgradeOpts {
    pub yes: bool,
    pub quiet: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AutoUpgradeState {
    last_attempt_unix: Option<i64>,
    last_success_unix: Option<i64>,
}

fn env_boolish(name: &str) -> Option<bool> {
    let v = std::env::var(name).ok()?;
    let v = v.trim().to_ascii_lowercase();
    if v.is_empty() {
        return None;
    }
    match v.as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn is_probably_dev_binary() -> bool {
    let Ok(p) = std::env::current_exe() else {
        return false;
    };
    // Avoid surprising upgrades when running from a repo build.
    let s = p.to_string_lossy().to_ascii_lowercase();
    s.contains("/target/") || s.contains("\\target\\")
}

fn auto_upgrade_interval_seconds() -> i64 {
    // Default: weekly, because the current installers may build from source (slow/heavy).
    let default_s: i64 = 7 * 24 * 60 * 60;
    let v = std::env::var("SEASHAIL_AUTO_UPGRADE_MIN_INTERVAL_SECONDS").ok();
    let Some(v) = v else { return default_s };
    let Ok(n) = v.trim().parse::<i64>() else {
        return default_s;
    };
    n.max(60)
}

fn should_auto_upgrade_by_default() -> bool {
    // Default to on for installed binaries, off for dev builds.
    !is_probably_dev_binary()
}

fn auto_upgrade_enabled() -> bool {
    if let Some(v) = env_boolish("SEASHAIL_DISABLE_AUTO_UPGRADE") {
        if v {
            return false;
        }
    }
    match env_boolish("SEASHAIL_AUTO_UPGRADE") {
        Some(true) => true,
        Some(false) => false,
        None => should_auto_upgrade_by_default(),
    }
}

fn state_path(paths: &SeashailPaths) -> PathBuf {
    paths.data_dir.join("auto-upgrade.json")
}

fn lock_path(paths: &SeashailPaths) -> PathBuf {
    paths.data_dir.join("auto-upgrade.lock")
}

fn read_state(path: &Path) -> eyre::Result<AutoUpgradeState> {
    if !path.exists() {
        return Ok(AutoUpgradeState::default());
    }
    let buf = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let st = serde_json::from_str::<AutoUpgradeState>(&buf)
        .with_context(|| format!("parse {}", path.display()))?;
    Ok(st)
}

fn write_state(path: &Path, st: &AutoUpgradeState) -> eyre::Result<()> {
    let mut s = serde_json::to_string_pretty(st).context("serialize auto-upgrade state")?;
    s.push('\n');
    crate::fsutil::write_string_atomic_restrictive(path, &s, crate::fsutil::MODE_FILE_PRIVATE)
}

fn temp_script_path(parent: &Path, ext: &str) -> PathBuf {
    let mut rand_bytes = [0_u8; 8];
    rand::rng().fill_bytes(&mut rand_bytes);
    let suffix = hex::encode(rand_bytes);
    parent.join(format!("seashail-installer-{suffix}.{ext}"))
}

async fn fetch_installer_bytes(url: &str) -> eyre::Result<Vec<u8>> {
    let client = reqwest::Client::new();
    let resp = client.get(url).send().await.context("fetch installer")?;
    let status = resp.status();
    if !status.is_success() {
        eyre::bail!("installer fetch failed with HTTP status {status}");
    }
    let bytes = resp.bytes().await.context("read installer bytes")?;
    // Safety valve: refuse anything too large.
    if bytes.len() > 2 * 1024 * 1024 {
        eyre::bail!(
            "installer script unexpectedly large ({} bytes)",
            bytes.len()
        );
    }
    Ok(bytes.to_vec())
}

fn resolve_installer_url() -> String {
    if cfg!(windows) {
        std::env::var("SEASHAIL_INSTALL_URL")
            .unwrap_or_else(|_| "https://seashail.com/install.ps1".into())
    } else {
        std::env::var("SEASHAIL_INSTALL_URL")
            .unwrap_or_else(|_| "https://seashail.com/install".into())
    }
}

const fn resolve_powershell_exes() -> [&'static str; 2] {
    // Prefer pwsh if available, else Windows PowerShell.
    ["pwsh", "powershell"]
}

fn is_homebrew_cellar_binary() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let Ok(p) = std::env::current_exe() else {
        return false;
    };
    let s = p.to_string_lossy();
    // Typical:
    // - /opt/homebrew/Cellar/seashail/<ver>/bin/seashail
    // - /usr/local/Cellar/seashail/<ver>/bin/seashail
    s.contains("/Cellar/seashail/")
}

fn resolve_upgrade_method() -> &'static str {
    // Override knob:
    // - SEASHAIL_UPGRADE_METHOD=brew|installer
    if let Ok(v) = std::env::var("SEASHAIL_UPGRADE_METHOD") {
        let v = v.trim().to_ascii_lowercase();
        if v == "brew" {
            return "brew";
        }
        if v == "installer" {
            return "installer";
        }
    }

    if is_homebrew_cellar_binary() {
        return "brew";
    }

    "installer"
}

async fn run_brew_upgrade(opts: UpgradeOpts) -> eyre::Result<()> {
    let brew = which::which("brew").context("missing dependency: brew")?;
    let mut cmd = tokio::process::Command::new(brew);
    cmd.arg("upgrade").arg("seashail");

    if opts.quiet {
        cmd.arg("--quiet");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    } else {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    }

    let status = cmd.status().await.context("brew upgrade seashail")?;
    if !status.success() {
        eyre::bail!("brew upgrade failed with exit code {:?}", status.code());
    }
    Ok(())
}

async fn run_installer(opts: UpgradeOpts, tmp_parent: &Path) -> eyre::Result<()> {
    let url = resolve_installer_url();
    let bytes = fetch_installer_bytes(&url).await?;

    if cfg!(windows) {
        let p = temp_script_path(tmp_parent, "ps1");
        // Best-effort restrictive write; Windows perms are different but we still use atomic pathing.
        crate::fsutil::write_atomic_restrictive(&p, &bytes, crate::fsutil::MODE_FILE_PRIVATE)
            .with_context(|| format!("write {}", p.display()))?;

        let mut last_err: Option<eyre::Report> = None;
        for ps in resolve_powershell_exes() {
            let mut cmd = tokio::process::Command::new(ps);
            cmd.arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(&p);

            if opts.quiet {
                cmd.stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
            } else {
                cmd.stdin(Stdio::null())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
            }

            match cmd.status().await {
                Ok(status) => {
                    drop(fs::remove_file(&p));
                    if !status.success() {
                        eyre::bail!("installer failed with exit code {:?}", status.code());
                    }
                    return Ok(());
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    last_err = Some(eyre::eyre!("missing dependency: {ps} ({e})"));
                }
                Err(e) => return Err(e).context("run powershell installer"),
            }
        }
        drop(fs::remove_file(&p));
        if let Some(e) = last_err {
            return Err(e);
        }
        eyre::bail!("missing dependency: powershell (or pwsh)");
    }

    let p = temp_script_path(tmp_parent, "sh");
    crate::fsutil::write_atomic_restrictive(&p, &bytes, 0o700_u32)
        .with_context(|| format!("write {}", p.display()))?;

    // Prefer bash (installer uses bash in the shebang) but fall back to sh.
    let shell = if which::which("bash").is_ok() {
        "bash"
    } else {
        "sh"
    };
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg(&p);

    if opts.quiet {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    } else {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    }

    let status = cmd.status().await.context("run installer")?;
    drop(fs::remove_file(&p));
    if !status.success() {
        eyre::bail!("installer failed with exit code {:?}", status.code());
    }
    Ok(())
}

async fn run_upgrade(opts: UpgradeOpts, paths: &SeashailPaths) -> eyre::Result<()> {
    match resolve_upgrade_method() {
        "brew" => run_brew_upgrade(opts).await,
        _ => run_installer(opts, &paths.data_dir).await,
    }
}

fn confirm_or_bail(yes: bool) -> eyre::Result<()> {
    crate::cli_output::confirm_upgrade_or_bail(yes)
}

pub async fn run(opts: UpgradeOpts) -> eyre::Result<()> {
    confirm_or_bail(opts.yes)?;
    let paths = SeashailPaths::discover().context("discover paths")?;
    // Use the private data dir as our temp parent to avoid /tmp races and to keep permissions strict.
    crate::fsutil::ensure_private_dir(&paths.data_dir)?;
    run_upgrade(opts, &paths).await
}

pub fn maybe_auto_upgrade(paths: &SeashailPaths) {
    if !auto_upgrade_enabled() {
        return;
    }

    let paths = paths.clone();
    // Background + best-effort: never block MCP startup on upgrades.
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }

    tokio::spawn(async move {
        if let Err(e) = auto_upgrade_task(&paths).await {
            tracing::debug!(error = %e, "auto-upgrade: skipped/failed");
        }
    });
}

async fn auto_upgrade_task(paths: &SeashailPaths) -> eyre::Result<()> {
    crate::fsutil::ensure_private_dir(&paths.data_dir)?;

    let lock_p = lock_path(paths);
    let lock_f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_p)
        .with_context(|| format!("open {}", lock_p.display()))?;

    // If another process is upgrading, just skip.
    if lock_f.try_lock_exclusive().is_err() {
        return Ok(());
    }

    let st_p = state_path(paths);
    let mut st = read_state(&st_p).unwrap_or_default();
    let now = Utc::now().timestamp();
    let interval = auto_upgrade_interval_seconds();

    if let Some(last) = st.last_attempt_unix {
        if now.saturating_sub(last) < interval {
            return Ok(());
        }
    }

    // Write attempt stamp first to avoid stampede across processes.
    st.last_attempt_unix = Some(now);
    write_state(&st_p, &st).ok();

    // Quiet + non-interactive.
    match run_upgrade(
        UpgradeOpts {
            yes: true,
            quiet: true,
        },
        paths,
    )
    .await
    {
        Ok(()) => {
            let now2 = Utc::now().timestamp();
            st.last_success_unix = Some(now2);
            write_state(&st_p, &st).ok();
            tracing::info!("auto-upgrade: completed");
            Ok(())
        }
        Err(e) => {
            // Don't persist detailed errors (may contain environment-specific paths).
            tracing::debug!(error = %e, "auto-upgrade: installer failed");
            Ok(())
        }
    }
}

// Minimal `which` implementation without pulling in a new dependency in multiple places.
// This crate is tiny and already common; add it once here.
mod which {
    pub fn which(name: &str) -> std::io::Result<std::path::PathBuf> {
        let paths = std::env::var_os("PATH").unwrap_or_default();
        for p in std::env::split_paths(&paths) {
            let candidate = p.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
            #[cfg(windows)]
            {
                for ext in ["exe", "cmd", "bat"] {
                    let c = p.join(format!("{name}.{ext}"));
                    if c.is_file() {
                        return Ok(c);
                    }
                }
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("not found: {name}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_boolish() {
        std::env::set_var("SEASHAIL_TEST_BOOL", "true");
        assert_eq!(env_boolish("SEASHAIL_TEST_BOOL"), Some(true));
        std::env::set_var("SEASHAIL_TEST_BOOL", "0");
        assert_eq!(env_boolish("SEASHAIL_TEST_BOOL"), Some(false));
        std::env::set_var("SEASHAIL_TEST_BOOL", "wat");
        assert_eq!(env_boolish("SEASHAIL_TEST_BOOL"), None);
        std::env::remove_var("SEASHAIL_TEST_BOOL");
        assert_eq!(env_boolish("SEASHAIL_TEST_BOOL"), None);
    }
}

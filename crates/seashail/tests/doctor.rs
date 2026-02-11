use std::process::Command;

use eyre::Context as _;

#[test]
fn doctor_json_runs_and_returns_valid_json() -> eyre::Result<()> {
    let exe = assert_cmd::cargo::cargo_bin!("seashail");

    let cfg_dir = tempfile::tempdir()?;
    let data_dir = tempfile::tempdir()?;

    let out = Command::new(exe)
        .env("SEASHAIL_CONFIG_DIR", cfg_dir.path())
        .env("SEASHAIL_DATA_DIR", data_dir.path())
        .args(["doctor", "--json"])
        .output()
        .context("run seashail doctor --json")?;

    assert!(
        out.status.success(),
        "doctor exited non-zero: status={:?}, stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).context("parse doctor json")?;
    assert_eq!(v.get("ok").and_then(serde_json::Value::as_bool), Some(true));
    assert!(v.get("version").and_then(|x| x.as_str()).is_some());
    assert!(v.get("paths").and_then(|x| x.as_object()).is_some());
    Ok(())
}

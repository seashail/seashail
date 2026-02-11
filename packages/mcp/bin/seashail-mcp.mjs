#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

function run(cmd, args, opts = {}) {
  return spawnSync(cmd, args, {
    stdio: "inherit",
    ...opts,
  });
}

function canRunSeashail() {
  const r = spawnSync("seashail", ["--version"], { stdio: "ignore" });
  return r.status === 0;
}

function resolveSeashailPath() {
  if (canRunSeashail()) return "seashail";

  const home = os.homedir();
  const exe = process.platform === "win32" ? ".exe" : "";
  const candidates = [
    process.env.SEASHAIL_BIN,
    path.join(home, ".local", "bin", `seashail${exe}`),
    path.join(home, ".cargo", "bin", `seashail${exe}`),
  ].filter(Boolean);

  for (const p of candidates) {
    try {
      fs.accessSync(p, fs.constants.X_OK);
      return p;
    } catch {
      // keep going
    }
  }

  return null;
}

function resolvePowerShell() {
  const r1 = spawnSync("pwsh", ["-NoProfile", "-Command", "$PSVersionTable"], {
    stdio: "ignore",
  });
  if (r1.status === 0) return "pwsh";
  return "powershell";
}

function installFromSource() {
  if (process.platform === "win32") {
    const ps = resolvePowerShell();
    const url =
      process.env.SEASHAIL_INSTALL_URL ?? "https://seashail.com/install.ps1";
    const cmd = `irm ${url} | iex`;
    const r = run(ps, ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", cmd], {
      env: process.env,
    });
    if (r.status !== 0) process.exit(r.status ?? 1);
    return;
  }

  const url = process.env.SEASHAIL_INSTALL_URL ?? "https://seashail.com/install";
  const cmd = `curl -fsSL ${url} | sh`;
  const r = run("sh", ["-c", cmd], { env: process.env });
  if (r.status !== 0) process.exit(r.status ?? 1);
}

const passthrough = process.argv.slice(2);

let seashail = resolveSeashailPath();
if (!seashail) {
  installFromSource();
  seashail = resolveSeashailPath();
}

if (!seashail) {
  console.error("seashail-mcp: failed to find the `seashail` binary after install.");
  process.exit(1);
}

const r = run(seashail, ["mcp", ...passthrough], {
  env: {
    // Default to enabling the binary's rate-limited auto-upgrade path.
    // Users can explicitly disable via SEASHAIL_AUTO_UPGRADE=0 or SEASHAIL_DISABLE_AUTO_UPGRADE=1.
    SEASHAIL_AUTO_UPGRADE: process.env.SEASHAIL_AUTO_UPGRADE ?? "1",
    ...process.env,
  },
});
process.exit(r.status ?? 1);

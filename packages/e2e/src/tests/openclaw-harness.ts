import assert from "node:assert/strict";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

type Json = null | boolean | number | string | Json[] | { [k: string]: Json };

function sanitizedParentEnv(): Record<string, string> {
  // OpenClaw + Seashail E2E must be isolated from developer shell state.
  // In particular, a globally-set `SEASHAIL_PASSPHRASE` would make “decline passphrase”
  // tests impossible to validate (the plugin would auto-answer prompts).
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(process.env)) {
    if (k.startsWith("SEASHAIL_") || k.startsWith("OPENCLAW_")) {
      continue;
    }
    // Keep E2E deterministic and allow loopback mocks to work even if the developer
    // shell has a proxy configured.
    if (
      k === "HTTP_PROXY" ||
      k === "http_proxy" ||
      k === "HTTPS_PROXY" ||
      k === "https_proxy" ||
      k === "ALL_PROXY" ||
      k === "all_proxy" ||
      k === "NO_PROXY" ||
      k === "no_proxy"
    ) {
      continue;
    }
    if (typeof v === "string") {
      out[k] = v;
    }
  }
  out["NO_PROXY"] = "127.0.0.1,localhost,[::1]";
  return out;
}

function readJsonFile(path: string): unknown {
  const raw = readFileSync(path, "utf8");
  return JSON.parse(raw) as unknown;
}

function writeJsonFile(path: string, v: unknown) {
  writeFileSync(path, `${JSON.stringify(v, null, 2)}\n`, "utf8");
}

function appendRing(dst: string, chunk: string, maxChars: number): string {
  const next = dst + chunk;
  if (next.length <= maxChars) {
    return next;
  }
  return next.slice(next.length - maxChars);
}

function randHex(bytes: number): string {
  // Avoid importing crypto; uniqueness is all we need for test isolation.
  const alphabet = "0123456789abcdef";
  let out = "";
  for (let i = 0; i < bytes * 2; i += 1) {
    out += alphabet[Math.floor(Math.random() * alphabet.length)];
  }
  return out;
}

async function pickFreePort(): Promise<number> {
  return await new Promise((resolve, reject) => {
    const s = net.createServer();
    s.on("error", reject);
    s.listen(0, "127.0.0.1", () => {
      const addr = s.address();
      if (!addr || typeof addr === "string") {
        s.close();
        reject(new Error("failed to resolve ephemeral port"));
        return;
      }
      const { port } = addr;
      s.close((err) => {
        if (err) {
          reject(err);
        } else {
          resolve(port);
        }
      });
    });
  });
}

/**
 * Copy the seashail binary into an isolated temp bin dir and prepend it to PATH in the given env.
 * Returns the path to the installed binary and mutates `env["PATH"]`.
 */
function installBinaryToTempDir(
  srcBinPath: string,
  tempBinDir: string,
  stateDir: string,
  env: Record<string, string>
): string {
  const binName = process.platform === "win32" ? "seashail.exe" : "seashail";
  const installed = join(tempBinDir, binName);
  try {
    mkdirSync(tempBinDir, { recursive: true });
    copyFileSync(srcBinPath, installed);
    if (process.platform !== "win32") {
      chmodSync(installed, 0o755);
    }
    const oldPath = env["PATH"] ?? process.env["PATH"] ?? "";
    const sep = process.platform === "win32" ? ";" : ":";
    env["PATH"] = `${tempBinDir}${sep}${oldPath}`;
  } catch (error) {
    rmSync(stateDir, { recursive: true, force: true });
    throw error;
  }
  return installed;
}

function safeJsonParse(s: string): unknown | null {
  try {
    return JSON.parse(s) as unknown;
  } catch {
    return null;
  }
}

function asRecord(v: unknown): Record<string, unknown> | null {
  if (!v || typeof v !== "object" || Array.isArray(v)) {
    return null;
  }
  return v as Record<string, unknown>;
}

function writeSeedSeashailConfigToml(
  seed: string | undefined,
  seashailConfigDir: string
) {
  if (typeof seed !== "string") {
    return;
  }
  writeFileSync(join(seashailConfigDir, "config.toml"), seed, "utf8");
}

function mergeOpenclawExtraConfig(
  base: Record<string, Json>,
  extra: Record<string, Json> | undefined
) {
  if (!extra) {
    return;
  }
  Object.assign(base, extra);
}

function enforceIsolatedWorkspace(
  base: Record<string, Json>,
  workspaceDir: string
) {
  // Enforce an isolated workspace per harness (otherwise OpenClaw may use ~/.openclaw/workspace).
  const agents = (asRecord(base["agents"]) ?? {}) as Record<string, Json>;
  const defaults = (asRecord(agents["defaults"]) ?? {}) as Record<string, Json>;
  defaults["workspace"] = workspaceDir as Json;
  agents["defaults"] = defaults as Json;
  base["agents"] = agents as Json;
}

function enforceGatewayConfig(
  base: Record<string, Json>,
  gatewayPort: number,
  gatewayToken: string
) {
  // Ensure gateway auth + port are deterministic for this harness.
  const gateway = (asRecord(base["gateway"]) ?? {}) as Record<string, Json>;
  gateway["port"] = gatewayPort as Json;
  gateway["auth"] = { mode: "token", token: gatewayToken } as Json;
  base["gateway"] = gateway as Json;
}

function ensureSeashailToolsAllowed(base: Record<string, Json>) {
  // Make sure any allowlist doesn't accidentally hide seashail tools.
  // (If the user config uses tool allowlists, we prefer to remove them for tests.)
  const tools = (asRecord(base["tools"]) ?? {}) as Record<string, Json>;
  // OpenClaw treats plugin tools as opt-in for direct invocation; enable Seashail additively.
  const alsoAllow = Array.isArray(tools["alsoAllow"])
    ? (tools["alsoAllow"] as Json[])
    : ([] as Json[]);
  if (!alsoAllow.some((v) => v === "seashail")) {
    alsoAllow.push("seashail");
  }
  tools["alsoAllow"] = alsoAllow as Json;
  delete tools["allow"];
  delete tools["deny"];
  // Ensure plugin is callable in sandboxed mode as well.
  const sandbox = (asRecord(tools["sandbox"]) ?? {}) as Record<string, Json>;
  const sandboxTools = (asRecord(sandbox["tools"]) ?? {}) as Record<
    string,
    Json
  >;
  const allow = Array.isArray(sandboxTools["allow"])
    ? (sandboxTools["allow"] as Json[])
    : ([] as Json[]);
  if (!allow.some((v) => v === "seashail")) {
    allow.push("seashail");
  }
  sandboxTools["allow"] = allow as Json;
  sandbox["tools"] = sandboxTools as Json;
  tools["sandbox"] = sandbox as Json;
  base["tools"] = tools as Json;
}

function buildOpenclawHarnessEnv(args: {
  stateDir: string;
  configPath: string;
}): Record<string, string> {
  return {
    ...sanitizedParentEnv(),
    OPENCLAW_STATE_DIR: args.stateDir,
    OPENCLAW_CONFIG_PATH: args.configPath,
  } as Record<string, string>;
}

function pickSeashailInstallBin(
  opts: { seashailBinPath: string; installSeashailToTempBin?: boolean },
  tempBinDir: string,
  stateDir: string,
  env: Record<string, string>
): string {
  const shouldInstall = opts.installSeashailToTempBin ?? true;
  if (!shouldInstall) {
    return opts.seashailBinPath;
  }
  return installBinaryToTempDir(
    opts.seashailBinPath,
    tempBinDir,
    stateDir,
    env
  );
}

function enableSeashailPluginBestEffort(
  openclawBin: string,
  env: Record<string, string>
) {
  // Best-effort; OpenClaw may already have it enabled from install.
  try {
    execFileSync(openclawBin, ["plugins", "enable", "seashail"], {
      env,
      stdio: "ignore",
    });
  } catch {
    // ignore
  }
}

function patchSeashailPluginConfigForIsolation(args: {
  configPath: string;
  standalone: boolean;
  prefix: string;
  seashailConfigDir: string;
  seashailDataDir: string;
  passphrase: string | undefined;
}) {
  // Ensure deterministic plugin env overrides for isolation (the Rust installer doesn't set these).
  const root = readJsonFile(args.configPath);
  const rootRec = (asRecord(root) ?? {}) as Record<string, Json>;
  const plugins = (asRecord(rootRec["plugins"]) ?? {}) as Record<string, Json>;
  const entries = (asRecord(plugins["entries"]) ?? {}) as Record<string, Json>;
  const seashailEntry = (asRecord(entries["seashail"]) ?? {}) as Record<
    string,
    Json
  >;
  const cfg = (asRecord(seashailEntry["config"]) ?? {}) as Record<string, Json>;
  cfg["standalone"] = args.standalone as Json;
  cfg["toolPrefix"] = true as Json;
  cfg["prefix"] = args.prefix as Json;
  cfg["passphraseEnvVar"] = "SEASHAIL_PASSPHRASE" as Json;
  cfg["env"] = {
    SEASHAIL_CONFIG_DIR: args.seashailConfigDir,
    SEASHAIL_DATA_DIR: args.seashailDataDir,
    SEASHAIL_DAEMON_IDLE_EXIT_SECONDS: "1",
    ...(typeof args.passphrase === "string" && args.passphrase.trim()
      ? { SEASHAIL_PASSPHRASE: args.passphrase }
      : {}),
  } as Json;
  seashailEntry["enabled"] = true as Json;
  seashailEntry["config"] = cfg as Json;
  entries["seashail"] = seashailEntry as Json;
  plugins["entries"] = entries as Json;
  rootRec["plugins"] = plugins as Json;
  writeJsonFile(args.configPath, rootRec);
}

export type ElicitReply =
  | { action: "accept"; content?: Record<string, unknown> }
  | { action: "decline"; content?: Record<string, unknown> };

export interface OpenClawHarnessOpts {
  // Absolute path to a seashail binary (usually workspace build output).
  seashailBinPath: string;
  openclawBinPath?: string;
  // Local OpenClaw plugin folder path (linked install).
  pluginPath: string;
  network?: "mainnet" | "testnet";
  passphrase?: string;
  keepTempDir?: boolean;
  // Shallow-merged into the generated openclaw.json before plugin install/enable.
  // Useful for tests that need extra OpenClaw config (e.g. models/providers for agent chat).
  openclawExtraConfig?: Record<string, Json>;
  // Controls `seashail openclaw install --onboard-wallet`.
  // Default: true (match CLI default).
  onboardWallet?: boolean;
  // If provided, written to `${seashailConfigDir}/config.toml` before any install/startup flows.
  // Use this for config that must apply during `seashail openclaw install` (e.g. short passphrase sessions).
  seedSeashailConfigToml?: string;
  // Override where Seashail stores its config/data (useful for multi-gateway tests).
  seashailConfigDir?: string;
  seashailDataDir?: string;
  // If true, the OpenClaw plugin will run `seashail mcp --standalone` instead of proxying to a daemon.
  standalone?: boolean;
}

export class OpenClawHarness {
  readonly stateDir: string;
  readonly configPath: string;
  gatewayPort: number;
  readonly gatewayToken: string;
  readonly seashailPrefix: string;
  readonly seashailConfigDir: string;
  readonly seashailDataDir: string;
  readonly env: Record<string, string>;

  private gatewayProc: ChildProcess | null = null;
  private gatewayStdout = "";
  private gatewayStderr = "";
  private readonly openclawBin: string;

  private constructor(args: {
    stateDir: string;
    configPath: string;
    gatewayPort: number;
    gatewayToken: string;
    seashailPrefix: string;
    seashailConfigDir: string;
    seashailDataDir: string;
    openclawBin: string;
    env: Record<string, string>;
  }) {
    this.stateDir = args.stateDir;
    this.configPath = args.configPath;
    this.gatewayPort = args.gatewayPort;
    this.gatewayToken = args.gatewayToken;
    this.seashailPrefix = args.seashailPrefix;
    this.seashailConfigDir = args.seashailConfigDir;
    this.seashailDataDir = args.seashailDataDir;
    this.openclawBin = args.openclawBin;
    this.env = args.env;
  }

  getGatewayLogs(): { stdout: string; stderr: string } {
    return { stdout: this.gatewayStdout, stderr: this.gatewayStderr };
  }

  getSeashailDiskLogs(): { seashailLogJsonl: string; auditJsonl: string } {
    const readSafe = (p: string): string => {
      try {
        return readFileSync(p, "utf8");
      } catch {
        return "";
      }
    };
    return {
      seashailLogJsonl: readSafe(
        join(this.seashailDataDir, "seashail.log.jsonl")
      ),
      auditJsonl: readSafe(join(this.seashailDataDir, "audit.jsonl")),
    };
  }

  static async create(opts: OpenClawHarnessOpts): Promise<OpenClawHarness> {
    // macOS `os.tmpdir()` paths are often long enough to break Unix socket paths (SUN_LEN).
    // Prefer /tmp on Unix to keep Seashail daemon socket paths short.
    const baseTmp = process.platform === "win32" ? tmpdir() : "/tmp";
    const stateDir = mkdtempSync(join(baseTmp, "openclaw-seashail-e2e-"));
    const configPath = join(stateDir, "openclaw.json");
    const gatewayPort = await pickFreePort();
    const gatewayToken = `seashail-e2e-${randHex(16)}`;
    const openclawBin = opts.openclawBinPath ?? "openclaw";
    const prefix = "seashail_";

    const seashailConfigDir =
      opts.seashailConfigDir ?? join(stateDir, "seashail/config");
    const seashailDataDir =
      opts.seashailDataDir ?? join(stateDir, "seashail/data");
    const workspaceDir = join(stateDir, "workspace");
    const standalone = opts.standalone === true;

    mkdirSync(seashailConfigDir, { recursive: true });
    mkdirSync(seashailDataDir, { recursive: true });
    mkdirSync(workspaceDir, { recursive: true });
    writeSeedSeashailConfigToml(opts.seedSeashailConfigToml, seashailConfigDir);

    // Base config: keep it minimal and deterministic for isolation ("reset openclaw").
    const base: Record<string, Json> = {
      gateway: {
        mode: "local",
        port: gatewayPort,
        auth: { mode: "token", token: gatewayToken },
      } as Json,
      agents: { defaults: { workspace: workspaceDir } } as Json,
      plugins: { entries: {} } as Json,
      tools: {
        profile: "full",
        sandbox: { tools: { allow: ["seashail"] } },
      } as Json,
    };
    mergeOpenclawExtraConfig(base, opts.openclawExtraConfig);
    enforceIsolatedWorkspace(base, workspaceDir);
    enforceGatewayConfig(base, gatewayPort, gatewayToken);
    ensureSeashailToolsAllowed(base);

    // Write config WITHOUT referencing the plugin yet. OpenClaw validates config before installs.
    writeJsonFile(configPath, base);

    // OpenClaw uses env vars to override state/config dirs. Use those for isolation.
    const harnessEnv = buildOpenclawHarnessEnv({
      stateDir,
      configPath,
    });

    // Install plugin into this isolated state dir.
    execFileSync(openclawBin, ["plugins", "install", "-l", opts.pluginPath], {
      env: harnessEnv,
      stdio: "pipe",
    });

    // Now that the plugin is installed/discovered, enable + configure it.
    const patched = readJsonFile(configPath);
    const patchedRec = (asRecord(patched) ?? {}) as Record<string, Json>;
    const plugins = (asRecord(patchedRec["plugins"]) ?? {}) as Record<
      string,
      Json
    >;
    const entries = (asRecord(plugins["entries"]) ?? {}) as Record<
      string,
      Json
    >;

    // Plugin config. We pass Seashail config/data dirs scoped under the OpenClaw state dir.
    const pluginCfg: Record<string, Json> = {
      seashailPath: opts.seashailBinPath as Json,
      network: (opts.network ?? "testnet") as Json,
      standalone: standalone as Json,
      toolPrefix: true as Json,
      prefix: prefix as Json,
      passphraseEnvVar: "SEASHAIL_PASSPHRASE" as Json,
      env: {
        SEASHAIL_CONFIG_DIR: seashailConfigDir,
        SEASHAIL_DATA_DIR: seashailDataDir,
        // Ensure the autostarted daemon doesn't linger after tests.
        SEASHAIL_DAEMON_IDLE_EXIT_SECONDS: "1",
        // Keep passphrase scoped to the Seashail subprocess, not the OpenClaw gateway.
        ...(typeof opts.passphrase === "string" && opts.passphrase.trim()
          ? { SEASHAIL_PASSPHRASE: opts.passphrase }
          : {}),
      } as Json,
    };
    entries["seashail"] = { enabled: true, config: pluginCfg } as Json;
    plugins["entries"] = entries as Json;
    patchedRec["plugins"] = plugins as Json;
    writeJsonFile(configPath, patchedRec);

    execFileSync(openclawBin, ["plugins", "enable", "seashail"], {
      env: harnessEnv,
      stdio: "pipe",
    });

    const h = new OpenClawHarness({
      stateDir,
      configPath,
      gatewayPort,
      gatewayToken,
      seashailPrefix: prefix,
      seashailConfigDir,
      seashailDataDir,
      openclawBin,
      env: harnessEnv,
    });

    await h.startGateway();
    return h;
  }

  static async createViaSeashailOpenclawInstall(
    opts: OpenClawHarnessOpts & {
      // If true, copy the provided `seashailBinPath` into an isolated bin dir and run the install via PATH.
      // This more closely matches a real user install.
      installSeashailToTempBin?: boolean;
    }
  ): Promise<OpenClawHarness> {
    const baseTmp = process.platform === "win32" ? tmpdir() : "/tmp";
    const stateDir = mkdtempSync(join(baseTmp, "openclaw-seashail-e2e-"));
    const configPath = join(stateDir, "openclaw.json");
    const gatewayPort = await pickFreePort();
    const gatewayToken = `seashail-e2e-${randHex(16)}`;
    const openclawBin = opts.openclawBinPath ?? "openclaw";
    const prefix = "seashail_";

    const seashailConfigDir =
      opts.seashailConfigDir ?? join(stateDir, "seashail/config");
    const seashailDataDir =
      opts.seashailDataDir ?? join(stateDir, "seashail/data");
    const workspaceDir = join(stateDir, "workspace");
    const standalone = opts.standalone === true;

    mkdirSync(seashailConfigDir, { recursive: true });
    mkdirSync(seashailDataDir, { recursive: true });
    mkdirSync(workspaceDir, { recursive: true });
    writeSeedSeashailConfigToml(opts.seedSeashailConfigToml, seashailConfigDir);

    // Base config first: OpenClaw validates config before plugin installs.
    const base: Record<string, Json> = {
      gateway: {
        mode: "local",
        port: gatewayPort,
        auth: { mode: "token", token: gatewayToken },
      } as Json,
      agents: { defaults: { workspace: workspaceDir } } as Json,
      plugins: { entries: {} } as Json,
      tools: {
        profile: "full",
        sandbox: { tools: { allow: ["seashail"] } },
      } as Json,
    };
    mergeOpenclawExtraConfig(base, opts.openclawExtraConfig);
    enforceIsolatedWorkspace(base, workspaceDir);
    enforceGatewayConfig(base, gatewayPort, gatewayToken);
    ensureSeashailToolsAllowed(base);
    writeJsonFile(configPath, base);

    const harnessEnv: Record<string, string> = {
      ...sanitizedParentEnv(),
      OPENCLAW_STATE_DIR: stateDir,
      OPENCLAW_CONFIG_PATH: configPath,
    } as Record<string, string>;

    // `seashail openclaw install` is now responsible for seamless onboarding, which may create
    // a default wallet. That flow is interactive by default, so for E2E we provide env-driven
    // answers during the install step only (not for the gateway runtime).
    const installEnv: Record<string, string> = {
      ...harnessEnv,
      SEASHAIL_CONFIG_DIR: seashailConfigDir,
      SEASHAIL_DATA_DIR: seashailDataDir,
      SEASHAIL_ACCEPT_DISCLAIMERS: "1",
    };

    const tempBinDir = join(stateDir, "bin");
    const seashailInstallBin = pickSeashailInstallBin(
      opts,
      tempBinDir,
      stateDir,
      harnessEnv
    );

    // Install + patch OpenClaw via `seashail openclaw install`.
    // Skip restarting any global gateway service: tests spawn their own `openclaw gateway`.
    execFileSync(
      seashailInstallBin,
      [
        "openclaw",
        "install",
        "--network",
        opts.network ?? "testnet",
        "--plugin",
        opts.pluginPath,
        "--link",
        "--openclaw-config-path",
        configPath,
        "--restart-gateway",
        "false",
        "--onboard-wallet",
        (opts.onboardWallet ?? true) ? "true" : "false",
      ],
      { env: installEnv, stdio: "pipe" }
    );

    patchSeashailPluginConfigForIsolation({
      configPath,
      standalone,
      prefix,
      seashailConfigDir,
      seashailDataDir,
      passphrase: opts.passphrase,
    });

    enableSeashailPluginBestEffort(openclawBin, harnessEnv);

    const h = new OpenClawHarness({
      stateDir,
      configPath,
      gatewayPort,
      gatewayToken,
      seashailPrefix: prefix,
      seashailConfigDir,
      seashailDataDir,
      openclawBin,
      env: harnessEnv,
    });
    await h.startGateway();
    return h;
  }

  async startGateway(): Promise<void> {
    if (this.gatewayProc) {
      return;
    }

    // Retry with a fresh port if the initial attempt fails (TOCTOU race between
    // pickFreePort and the gateway actually binding the socket).
    const maxAttempts = 3;
    let lastError: Error = new Error("gateway failed to start");
    for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
      if (attempt > 0) {
        // Pick a new port and update the on-disk config so the gateway reads it.
        this.gatewayPort = await pickFreePort();
        const cfg = readJsonFile(this.configPath);
        const cfgRec = (asRecord(cfg) ?? {}) as Record<string, Json>;
        enforceGatewayConfig(cfgRec, this.gatewayPort, this.gatewayToken);
        writeJsonFile(this.configPath, cfgRec);
        this.gatewayStdout = "";
        this.gatewayStderr = "";
      }

      const p = spawn(
        this.openclawBin,
        // `--force` avoids flaky port reuse races between tests if a prior gateway didn't
        // terminate promptly (or the OS reallocated the same ephemeral port).
        ["gateway", "run", "--force", "--port", String(this.gatewayPort)],
        {
          env: this.env,
          stdio: ["ignore", "pipe", "pipe"],
          // OpenClaw spawns a separate `openclaw-gateway` process. Ensure we can kill the whole
          // process group on shutdown so we don't leak listeners between tests.
          detached: process.platform !== "win32",
        }
      );
      this.gatewayProc = p;
      p.stdout?.setEncoding("utf8");
      p.stderr?.setEncoding("utf8");
      p.stdout?.on("data", (d) => {
        this.gatewayStdout = appendRing(this.gatewayStdout, String(d), 200_000);
      });
      p.stderr?.on("data", (d) => {
        this.gatewayStderr = appendRing(this.gatewayStderr, String(d), 200_000);
      });

      try {
        await this.waitForGatewayReady(30_000);
        return;
      } catch (error) {
        lastError = error instanceof Error ? error : new Error(String(error));
        // Clean up the failed process before retrying.
        this.gatewayProc = null;
        try {
          if (p.exitCode === null) {
            p.kill("SIGKILL");
          }
        } catch {
          // ignore
        }
      }
    }
    throw lastError;
  }

  async stop(): Promise<void> {
    if (!this.gatewayProc) {
      return;
    }
    const p = this.gatewayProc;
    this.gatewayProc = null;

    if (p.exitCode !== null) {
      return;
    }

    const exitPromise = new Promise<void>((resolve) => {
      p.on("exit", () => resolve());
    });

    try {
      if (process.platform !== "win32" && typeof p.pid === "number") {
        // Kill the entire process group (OpenClaw CLI + spawned gateway).
        process.kill(-p.pid, "SIGTERM");
      } else {
        p.kill("SIGTERM");
      }
    } catch {
      return;
    }

    const timeout = setTimeout(() => {
      try {
        if (p.exitCode === null) {
          if (process.platform !== "win32" && typeof p.pid === "number") {
            process.kill(-p.pid, "SIGKILL");
          } else {
            p.kill("SIGKILL");
          }
        }
      } catch {
        // ignore
      }
    }, 2000);

    await exitPromise;
    clearTimeout(timeout);
  }

  async cleanup(opts?: { keepTempDir?: boolean }): Promise<void> {
    await this.stop();
    try {
      // OpenClaw does not currently support uninstall; disabling is the closest equivalent.
      execFileSync(this.openclawBin, ["plugins", "disable", "seashail"], {
        env: this.env,
        stdio: "ignore",
        // Avoid hanging test cleanup on flaky OpenClaw CLI behavior.
        timeout: 1500,
      });
    } catch {
      // ignore
    }
    if (opts?.keepTempDir) {
      return;
    }
    rmSync(this.stateDir, { recursive: true, force: true });
  }

  private async waitForGatewayReady(timeoutMs: number): Promise<void> {
    const started = Date.now();
    while (Date.now() - started < timeoutMs) {
      if (this.gatewayProc && this.gatewayProc.exitCode !== null) {
        throw new Error(
          `openclaw gateway exited early (code=${this.gatewayProc.exitCode}).\n\nSTDERR:\n${this.gatewayStderr}\n\nSTDOUT:\n${this.gatewayStdout}`
        );
      }
      try {
        const resp = await fetch(
          `http://127.0.0.1:${this.gatewayPort}/health`,
          {
            headers: { Authorization: `Bearer ${this.gatewayToken}` },
          }
        );
        if (resp.ok) {
          return;
        }
      } catch {
        // ignore; retry
      }
      await new Promise<void>((resolve) => {
        setTimeout(resolve, 200);
      });
    }
    throw new Error(
      `openclaw gateway did not become ready within ${timeoutMs}ms`
    );
  }

  async invokeTool(
    tool: string,
    args: Record<string, unknown> = {},
    action?: string
  ): Promise<unknown> {
    const resp = await fetch(
      `http://127.0.0.1:${this.gatewayPort}/tools/invoke`,
      {
        method: "POST",
        headers: {
          Authorization: `Bearer ${this.gatewayToken}`,
          "content-type": "application/json",
        },
        body: JSON.stringify({
          tool,
          ...(action ? { action } : {}),
          args,
        }),
      }
    );

    const text = await resp.text();
    const parsed = safeJsonParse(text);
    if (!parsed || typeof parsed !== "object") {
      throw new Error(
        `tools/invoke returned non-json (status=${resp.status}): ${text.slice(0, 500)}`
      );
    }

    const rec = parsed as Record<string, unknown>;
    if (rec["ok"] !== true) {
      throw new Error(
        `tools/invoke failed (status=${resp.status}): ${text.slice(0, 1000)}`
      );
    }
    return rec["result"];
  }

  async invokeSeashailTool(
    toolName: string,
    args: Record<string, unknown>,
    handler: (message: string) => ElicitReply = () => ({ action: "accept" })
  ): Promise<{
    ok: boolean;
    payload: unknown;
    rawText: string;
    details: Record<string, unknown>;
  }> {
    const fullName = `${this.seashailPrefix}${toolName}`;
    const resumeName = `${this.seashailPrefix}resume`;

    let res = await this.invokeTool(fullName, args);
    for (let i = 0; i < 25; i += 1) {
      const resRec = asRecord(res) ?? {};
      const details = (asRecord(resRec["details"]) ?? {}) as Record<
        string,
        unknown
      >;

      const content = Array.isArray(resRec["content"])
        ? (resRec["content"] as unknown[])
        : [];
      const first = content.length > 0 ? asRecord(content[0]) : null;
      const rawText =
        first && first["type"] === "text" ? String(first["text"] ?? "") : "";

      const { status } = details;
      if (status === "needs_approval") {
        const token =
          typeof details["token"] === "string" ? details["token"] : "";
        const message =
          typeof details["message"] === "string" ? details["message"] : rawText;
        assert.ok(token, "missing resume token");
        const reply = handler(message);
        res = await this.invokeTool(resumeName, {
          token,
          action: reply.action,
          ...(reply.content ? { content: reply.content } : {}),
        });
        // continue loop, the resumed call may yield again
        continue;
      }

      const ok = details["ok"] === true;
      const payload = details["payload"] ?? safeJsonParse(rawText) ?? rawText;
      return { ok, payload, rawText, details };
    }
    throw new Error(`too many resume loops calling ${toolName}`);
  }
}

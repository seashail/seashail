import assert from "node:assert/strict";
import { execFileSync, execSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";

import {
  makeDefaultElicitationHandler,
  makeTestPassphrase,
} from "./elicitation";
import { OpenClawHarness, type ElicitReply } from "./openclaw-harness";

const PASSPHRASE = makeTestPassphrase();
const handleElicitation = makeDefaultElicitationHandler(PASSPHRASE) as (
  message: string
) => ElicitReply;

function parseArgs(argv: string[]) {
  const out = {
    keep: false as boolean,
    legacy: false as boolean,
    standalone: false as boolean,
    chat: false as boolean,
  };
  for (const a of argv) {
    if (a === "--keep") {
      out.keep = true;
    }
    if (a === "--legacy") {
      out.legacy = true;
    }
    if (a === "--standalone") {
      out.standalone = true;
    }
    if (a === "--chat") {
      out.chat = true;
    }
  }
  return out;
}

function stripAnsiSgr(s: string): string {
  // Remove common SGR ANSI sequences: ESC [ ... m
  // Avoid regex literals containing control characters (eslint no-control-regex).
  const ESC = 27;
  let out = "";
  for (let i = 0; i < s.length; i += 1) {
    if (s.codePointAt(i) === ESC && s[i + 1] === "[") {
      let j = i + 2;
      while (j < s.length) {
        const code = s.codePointAt(j);
        if (code !== undefined && ((code >= 48 && code <= 57) || code === 59)) {
          j += 1;
          continue;
        }
        if (s[j] === "m") {
          j += 1;
        }
        break;
      }
      i = j - 1;
      continue;
    }
    out += s[i] as string;
  }
  return out;
}

function extractPrintable(s: string): string {
  return stripAnsiSgr(s);
}

function extractFirstJsonObject(out: string): any | null {
  const clean = extractPrintable(out);
  const i = clean.indexOf("{");
  const j = clean.lastIndexOf("}");
  if (i === -1 || j === -1 || j <= i) {
    return null;
  }
  try {
    return JSON.parse(clean.slice(i, j + 1)) as any;
  } catch {
    return null;
  }
}

function readUserOpenClawConfig(): any | null {
  try {
    const home = process.env["HOME"] ?? "";
    const p =
      process.env["OPENCLAW_CONFIG_PATH"] ?? `${home}/.openclaw/openclaw.json`;
    const raw = readFileSync(p, "utf8");
    return JSON.parse(raw) as any;
  } catch {
    return null;
  }
}

function buildSeashailBinary(): string {
  execSync("cargo build -p seashail", {
    cwd: new URL("../../../../", import.meta.url).pathname,
    timeout: 120_000,
    stdio: "pipe",
  });

  const seashailBin = new URL(
    "../../../../target/debug/seashail",
    import.meta.url
  ).pathname;
  assert.ok(
    existsSync(seashailBin),
    `missing seashail binary at ${seashailBin}`
  );
  return seashailBin;
}

function resolvePluginPath(): string {
  return new URL("../../../openclaw-seashail-plugin", import.meta.url).pathname;
}

function buildOpenclawExtraConfig(
  args: ReturnType<typeof parseArgs>
): any | undefined {
  if (!args.chat) {
    return undefined;
  }
  const userCfg = readUserOpenClawConfig();
  if (!userCfg) {
    return undefined;
  }
  // For chat E2E we need the user's real OpenClaw model/provider config; the harness config is
  // intentionally minimal. We'll best-effort merge `agents` + `models` from the user's config.
  return { agents: userCfg.agents, models: userCfg.models } as any;
}

async function createHarness(
  args: ReturnType<typeof parseArgs>,
  seashailBin: string
) {
  const pluginPath = resolvePluginPath();
  const openclawExtraConfig = buildOpenclawExtraConfig(args);

  if (args.legacy) {
    return await OpenClawHarness.create({
      seashailBinPath: seashailBin,
      pluginPath,
      network: "testnet",
      standalone: args.standalone,
      openclawExtraConfig,
    });
  }

  return await OpenClawHarness.createViaSeashailOpenclawInstall({
    seashailBinPath: seashailBin,
    pluginPath,
    network: "testnet",
    standalone: args.standalone,
    openclawExtraConfig,
  });
}

async function runSeashailToolSmoke(h: OpenClawHarness) {
  const caps = await h.invokeSeashailTool(
    "get_capabilities",
    {},
    handleElicitation
  );
  assert.equal(caps.ok, true, "get_capabilities failed");

  // Default wallet should already exist (auto-generated on first use).
  const listed = await h.invokeSeashailTool(
    "list_wallets",
    {},
    handleElicitation
  );
  assert.equal(listed.ok, true, "list_wallets failed");

  // Deposit addresses (no QR; address-only).
  const solDep = await h.invokeSeashailTool(
    "get_deposit_info",
    { chain: "solana" },
    handleElicitation
  );
  assert.equal(solDep.ok, true, "get_deposit_info solana failed");

  const evmDep = await h.invokeSeashailTool(
    "get_deposit_info",
    { chain: "ethereum" },
    handleElicitation
  );
  assert.equal(evmDep.ok, true, "get_deposit_info ethereum failed");

  const btcDep = await h.invokeSeashailTool(
    "get_deposit_info",
    { chain: "bitcoin" },
    handleElicitation
  );
  assert.equal(btcDep.ok, true, "get_deposit_info bitcoin failed");

  const faucet = await h.invokeSeashailTool(
    "get_testnet_faucet_links",
    {
      chain: "sepolia",
      address: "0x000000000000000000000000000000000000dEaD",
    },
    handleElicitation
  );
  assert.equal(faucet.ok, true, "get_testnet_faucet_links failed");

  return { solDep, evmDep, btcDep };
}

function runRealOpenClawAgentChatIfEnabled(
  args: ReturnType<typeof parseArgs>,
  h: OpenClawHarness,
  deps: { solDep: any; evmDep: any; btcDep: any }
): void {
  if (!args.chat) {
    return;
  }

  // Real OpenClaw agent chat: require it to use Seashail tools and repeat the deposit
  // addresses (ground truth above). This is a strong signal that tools were actually called.
  const wantSol = String((deps.solDep.payload as any)?.address);
  const wantEvm = String((deps.evmDep.payload as any)?.address);
  const wantBtc = String((deps.btcDep.payload as any)?.address);
  assert.ok(wantSol.length > 10, "missing solana deposit address");
  assert.ok(wantEvm.startsWith("0x"), "missing evm deposit address");
  assert.ok(wantBtc.length > 10, "missing bitcoin deposit address");

  const out = execFileSync(
    "openclaw",
    [
      "agent",
      "--session-id",
      `seashail-openclaw-real-${Date.now()}`,
      "--message",
      [
        "Use seashail tools (not memory) to fetch deposit addresses for:",
        "- Solana",
        "- Ethereum",
        "- Bitcoin",
        "",
        "Return a single JSON object with keys solana, ethereum, bitcoin, each containing the address string.",
        "Do not invent addresses.",
      ].join("\n"),
      "--json",
      "--timeout",
      "120",
      "--thinking",
      "off",
    ],
    { env: h.env, stdio: "pipe" }
  ).toString("utf8");

  const parsed = extractFirstJsonObject(out);
  assert.ok(
    parsed && typeof parsed === "object",
    "openclaw agent did not return json output"
  );
  assert.equal((parsed as any).status, "ok", "openclaw agent status != ok");
  assert.equal(
    (parsed as any).result?.meta?.aborted,
    false,
    "openclaw agent run aborted"
  );
  const textVal = (parsed as any).result?.payloads?.[0]?.text;
  assert.ok(
    typeof textVal === "string" && textVal.length > 0,
    "openclaw agent returned empty payload"
  );

  const jsonOut = extractFirstJsonObject(textVal);
  assert.ok(
    jsonOut && typeof jsonOut === "object",
    "agent did not output json object"
  );
  const gotSol = String((jsonOut as any).solana);
  const gotEvm = String((jsonOut as any).ethereum);
  const gotBtc = String((jsonOut as any).bitcoin);
  assert.equal(
    gotSol,
    wantSol,
    "solana address mismatch (agent likely did not use tool)"
  );
  assert.equal(
    gotEvm,
    wantEvm,
    "evm address mismatch (agent likely did not use tool)"
  );
  assert.equal(
    gotBtc,
    wantBtc,
    "bitcoin address mismatch (agent likely did not use tool)"
  );
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const seashailBin = buildSeashailBinary();
  const h = await createHarness(args, seashailBin);

  try {
    const deps = await runSeashailToolSmoke(h);
    runRealOpenClawAgentChatIfEnabled(args, h, deps);

    // Minimal signal to callers/CI.
    // (Do not print secrets; Seashail payloads can contain sensitive data.)
    const summary: Record<string, unknown> = {
      ok: true,
      gatewayPort: h.gatewayPort,
    };
    if (args.keep) {
      summary["stateDir"] = h.stateDir;
    }
    process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
  } finally {
    await h.cleanup({ keepTempDir: args.keep });
  }
}

try {
  await main();
} catch (error: unknown) {
  const msg =
    error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  process.stderr.write(`${msg}\n`);
  process.exit(1);
}

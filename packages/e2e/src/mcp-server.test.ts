import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import assert from "node:assert/strict";
import { execFileSync, execSync, spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync, mkdtempSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

type JsonRpcId = string | number;

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: JsonRpcId;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

interface JsonRpcRequest {
  jsonrpc: "2.0";
  id?: JsonRpcId;
  method: string;
  params?: unknown;
}

beforeAll(() => {
  // E2E tests spawn `target/debug/seashail` directly.
  execSync("cargo build -p seashail", {
    cwd: new URL("../../../", import.meta.url).pathname,
    timeout: 180_000,
    stdio: "pipe",
  });
}, 180_000);

afterAll(() => {
  // no-op
});

function hasCmd(cmd: string, args: string[] = ["--version"]): boolean {
  try {
    execFileSync(cmd, args, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

const HAVE_ANVIL = hasCmd("anvil", ["--version"]);

function delayMs(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

async function retry<T>(
  label: string,
  fn: () => Promise<T>,
  opts?: { tries?: number; delayMs?: number }
): Promise<T> {
  const tries = opts?.tries ?? 5;
  const backoffMs = opts?.delayMs ?? 600;
  let lastErr: unknown = null;
  for (let i = 0; i < tries; i += 1) {
    try {
      return await fn();
    } catch (error) {
      lastErr = error;
      await delayMs(backoffMs * (i + 1));
    }
  }
  throw new Error(`${label} failed after ${tries} tries: ${String(lastErr)}`);
}

function writeJsonLine(child: ReturnType<typeof spawn>, msg: unknown) {
  assert.ok(child.stdin, "expected piped stdin");
  child.stdin.write(`${JSON.stringify(msg)}\n`);
}

async function nextJsonLine(
  p: ReturnType<typeof spawn>,
  buffer: { text: string }
): Promise<JsonRpcRequest | JsonRpcResponse | null> {
  const { stdout } = p;
  if (!stdout) {
    throw new Error("expected piped stdout");
  }

  const chunk = await new Promise<string | null>((resolve) => {
    const onData = (d: Buffer) => {
      cleanup();
      resolve(d.toString("utf8"));
    };
    const onEnd = () => {
      cleanup();
      resolve(null);
    };
    const cleanup = () => {
      stdout.off("data", onData);
      stdout.off("end", onEnd);
    };
    stdout.on("data", onData);
    stdout.on("end", onEnd);
  });

  if (chunk === null) {
    return null;
  }

  buffer.text += chunk;
  const idx = buffer.text.indexOf("\n");
  if (idx === -1) {
    return nextJsonLine(p, buffer);
  }

  const line = buffer.text.slice(0, idx).trim();
  buffer.text = buffer.text.slice(idx + 1);
  if (!line) {
    return nextJsonLine(p, buffer);
  }
  return JSON.parse(line) as JsonRpcRequest | JsonRpcResponse;
}

function extractShareFromMessage(message: string): string {
  const marker = "SHARE3_BASE64:\n";
  const i = message.indexOf(marker);
  if (i === -1) {
    return "";
  }
  const rest = message.slice(i + marker.length);
  return rest.split("\n")[0]?.trim() ?? "";
}

interface ElicitReply {
  action: string;
  content?: Record<string, unknown>;
}

const DEFAULT_PASSPHRASE = `seashail-e2e-${randomBytes(18).toString("hex")}`;

function defaultElicitationHandler(message: string): ElicitReply {
  if (message.includes("Set a Seashail passphrase")) {
    return {
      action: "accept",
      content: { passphrase: DEFAULT_PASSPHRASE },
    };
  }
  if (message.includes("Enter your Seashail passphrase")) {
    return {
      action: "accept",
      content: { passphrase: DEFAULT_PASSPHRASE },
    };
  }
  if (message.includes("Offline backup share")) {
    const share3 = extractShareFromMessage(message);
    const tail = share3.slice(-6);
    return { action: "accept", content: { confirm_tail: tail, ack: true } };
  }
  if (message.startsWith("Disclaimers:")) {
    return { action: "accept", content: { accept: true } };
  }
  if (
    message.startsWith("Seashail policy requires confirmation.") ||
    message.startsWith("Seashail requires confirmation.")
  ) {
    return { action: "accept", content: { confirm: true } };
  }
  if (message.includes("Confirm wallet import.")) {
    return { action: "accept", content: { confirm: true } };
  }
  return { action: "decline" };
}

function elicitationHandlerWithSecret(
  secret: string
): (message: string) => ElicitReply {
  return (message: string) => {
    if (message.includes("Paste your private key")) {
      return { action: "accept", content: { secret } };
    }
    if (message.includes("Paste your mnemonic")) {
      return { action: "accept", content: { secret } };
    }
    return defaultElicitationHandler(message);
  };
}

async function driveElicitationWithHandler(
  p: ReturnType<typeof spawn>,
  outBuf: { text: string },
  targetId: number,
  handler: (message: string) => ElicitReply
): Promise<Map<JsonRpcId, JsonRpcResponse>> {
  const responses = new Map<JsonRpcId, JsonRpcResponse>();

  for (;;) {
    const msg = await nextJsonLine(p, outBuf);
    assert.ok(msg, "expected a message");

    if (msg && typeof msg === "object" && "method" in msg) {
      const req = msg as JsonRpcRequest;
      if (req.method === "elicitation/create") {
        const id = req.id as JsonRpcId;
        const params = (req.params ?? {}) as Record<string, unknown>;
        const message = String(params["message"] ?? "");
        writeJsonLine(p, { jsonrpc: "2.0", id, result: handler(message) });
        continue;
      }
    }

    if (msg && typeof msg === "object" && "id" in msg) {
      const res = msg as JsonRpcResponse;
      if (res.jsonrpc === "2.0") {
        responses.set(res.id, res);
      }
    }

    if (responses.get(targetId)) {
      return responses;
    }
  }
}

function parseToolPayload(res: JsonRpcResponse): {
  isError: boolean;
  payload: any;
} {
  const raw = res.result;
  if (!raw || typeof raw !== "object") {
    return { isError: false, payload: raw };
  }
  const result = raw as Record<string, unknown>;
  const { content, isError: isErrorValue } = result;
  const isError = isErrorValue === true;
  const first =
    Array.isArray(content) &&
    typeof content[0] === "object" &&
    content[0] !== null
      ? (content[0] as Record<string, unknown>)
      : undefined;
  const text = String(first?.["text"] ?? "");
  try {
    return { isError, payload: JSON.parse(text) };
  } catch {
    return { isError, payload: text };
  }
}

function mustToolOk(res: JsonRpcResponse, label: string): any {
  const p = parseToolPayload(res);
  if (p.isError) {
    throw new TypeError(`${label} failed: ${JSON.stringify(p.payload)}`);
  }
  return p.payload;
}

function extractSurfaces(snapshots: any[]): string[] {
  return snapshots
    .filter(
      (s: any) => s && typeof s === "object" && typeof s.surface === "string"
    )
    .map((s: any) => s.surface as string);
}

async function rpcCall(
  p: ReturnType<typeof spawn>,
  outBuf: { text: string },
  id: number,
  method: string,
  params: unknown
): Promise<JsonRpcResponse> {
  writeJsonLine(p, { jsonrpc: "2.0", id, method, params });
  for (;;) {
    const msg = await nextJsonLine(p, outBuf);
    assert.ok(msg, "expected a message");

    if (msg && typeof msg === "object" && "method" in msg) {
      const req = msg as JsonRpcRequest;
      if (req.method === "elicitation/create") {
        const eid = req.id as JsonRpcId;
        const eparams = (req.params ?? {}) as Record<string, unknown>;
        const message = String(eparams["message"] ?? "");
        writeJsonLine(p, {
          jsonrpc: "2.0",
          id: eid,
          result: defaultElicitationHandler(message),
        });
        continue;
      }
    }

    if (msg && typeof msg === "object" && "id" in msg) {
      const res = msg as JsonRpcResponse;
      if (res.id === id) {
        return res;
      }
    }
  }
}

async function solanaRpc(url: string, method: string, params: unknown[]) {
  const resp = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const json = (await resp.json()) as any;
  if (json.error) {
    throw new Error(`solana rpc error: ${JSON.stringify(json.error)}`);
  }
  return json.result;
}

async function anvilRpc(url: string, method: string, params: unknown[]) {
  const resp = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const json = (await resp.json()) as any;
  if (json.error) {
    throw new Error(`anvil rpc error: ${JSON.stringify(json.error)}`);
  }
  return json.result;
}

async function waitForAnvil(url: string): Promise<void> {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    try {
      await anvilRpc(url, "eth_chainId", []);
      return;
    } catch {
      // retry
    }
    await delayMs(200);
  }
  throw new Error("anvil did not start");
}

async function waitForReceipt(anvilUrl: string, txHash: string): Promise<any> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    const r = await anvilRpc(anvilUrl, "eth_getTransactionReceipt", [txHash]);
    if (r) {
      return r;
    }
    await delayMs(100);
  }
  throw new Error("timed out waiting for receipt");
}

function pickFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
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
      s.close();
      resolve(port);
    });
  });
}

async function pickFreeSolanaPorts(): Promise<{
  rpcPort: number;
  gossipPort: number;
  faucetPort: number;
}> {
  for (let i = 0; i < 40; i += 1) {
    const rpcPort = await pickFreePort();
    const wsPort = rpcPort + 1;

    // Ensure wsPort is available.
    await new Promise<void>((resolve, reject) => {
      const s = net.createServer();
      s.on("error", reject);
      s.listen(wsPort, "127.0.0.1", () => {
        s.close();
        s.once("close", resolve);
      });
    });

    let gossipPort = await pickFreePort();
    while (gossipPort === rpcPort || gossipPort === wsPort) {
      gossipPort = await pickFreePort();
    }

    let faucetPort = await pickFreePort();
    while (
      faucetPort === rpcPort ||
      faucetPort === wsPort ||
      faucetPort === gossipPort
    ) {
      faucetPort = await pickFreePort();
    }

    return { rpcPort, gossipPort, faucetPort };
  }
  throw new Error("unable to find free solana port set");
}

function tailPipe(
  stream: NodeJS.ReadableStream | null | undefined,
  buf: { text: string },
  limit = 16_000
) {
  if (!stream) {
    return;
  }
  stream.on("data", (d: Buffer) => {
    buf.text = (buf.text + d.toString("utf8")).slice(-limit);
  });
}

async function waitForSolana(url: string): Promise<void> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    try {
      await solanaRpc(url, "getHealth", []);
      return;
    } catch {
      // retry
    }
    await delayMs(200);
  }
  throw new Error("solana-test-validator did not start");
}

async function waitForSolanaBalance(
  url: string,
  pubkey: string,
  minLamports: number
): Promise<void> {
  for (let i = 0; i < 400; i += 1) {
    const res = (await solanaRpc(url, "getBalance", [
      pubkey,
      { commitment: "confirmed" },
    ])) as { value?: number };
    const v = res.value ?? 0;
    if (v >= minLamports) {
      return;
    }
    await delayMs(50);
  }
  throw new Error("airdrop did not arrive in time");
}

async function getSolanaBalanceConfirmed(
  url: string,
  pubkey: string
): Promise<number> {
  const res = (await solanaRpc(url, "getBalance", [
    pubkey,
    { commitment: "confirmed" },
  ])) as { value?: number };
  return res.value ?? 0;
}

async function waitForSolanaBalanceBelow(
  url: string,
  pubkey: string,
  belowLamports: number
): Promise<number> {
  for (let i = 0; i < 400; i += 1) {
    const v = await getSolanaBalanceConfirmed(url, pubkey);
    if (v < belowLamports) {
      return v;
    }
    await delayMs(50);
  }
  throw new Error("solana send did not reflect in balance in time");
}

const LIVE = process.env["SEASHAIL_E2E_LIVE"] === "1";
// Default-off: these hit live public endpoints (mainnet RPC / upstream APIs) and are not CI-safe.
const liveDescribe = LIVE ? describe : describe.skip;

liveDescribe("seashail MCP stdio (live reads)", () => {
  test("pump.fun read tools use high-precision RPC fallback by default", async () => {
    const dataDir = mkdtempSync(join(tmpdir(), "seashail-e2e-data-"));
    const configDir = mkdtempSync(join(tmpdir(), "seashail-e2e-config-"));

    const bin = new URL("../../../target/debug/seashail", import.meta.url)
      .pathname;
    expect(existsSync(bin)).toBe(true);

    const p = spawn(bin, ["mcp"], {
      cwd: new URL("../../../", import.meta.url).pathname,
      stdio: ["pipe", "pipe", "pipe"],
      env: {
        ...process.env,
        RUST_LOG: "warn",
        SEASHAIL_DATA_DIR: dataDir,
        SEASHAIL_CONFIG_DIR: configDir,
      },
    });

    const outBuf = { text: "" };
    await rpcCall(p, outBuf, 1, "initialize", {});

    // Create a wallet so wallet-dependent setup doesn't interfere later.
    writeJsonLine(p, {
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: { name: "create_wallet", arguments: { name: "pf" } },
    });
    await driveElicitationWithHandler(p, outBuf, 2, defaultElicitationHandler);

    const listObj = await retry(
      "pumpfun_list_new_coins",
      async () => {
        const listRes = await rpcCall(p, outBuf, 3, "tools/call", {
          name: "pumpfun_list_new_coins",
          arguments: { limit: 1 },
        });
        return mustToolOk(listRes, "pumpfun_list_new_coins");
      },
      { tries: 6, delayMs: 800 }
    );

    expect(listObj.source).toBe("rpc");
    expect(listObj.program_id).toBe(
      "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"
    );
    expect(Array.isArray(listObj.items)).toBe(true);
    expect(listObj.items.length).toBeGreaterThan(0);

    const [first] = listObj.items;
    expect(typeof first.mint).toBe("string");
    expect(typeof first.bonding_curve).toBe("string");
    expect(typeof first.curve).toBe("object");

    const infoObj = await retry(
      "pumpfun_get_coin_info",
      async () => {
        const infoRes = await rpcCall(p, outBuf, 4, "tools/call", {
          name: "pumpfun_get_coin_info",
          arguments: { mint: first.mint },
        });
        return mustToolOk(infoRes, "pumpfun_get_coin_info");
      },
      { tries: 6, delayMs: 800 }
    );

    expect(infoObj.source).toBe("rpc");
    expect(infoObj.mint).toBe(first.mint);
    expect(infoObj.bonding_curve).toBe(first.bonding_curve);
    expect(typeof infoObj.curve).toBe("object");

    p.stdin?.end();
    await new Response(p.stderr).text();
  }, 120_000);

  test("health snapshots: lending + prediction live reads persist and surface via get_portfolio(include_health)", async () => {
    const dataDir = mkdtempSync(join(tmpdir(), "seashail-e2e-data-"));
    const configDir = mkdtempSync(join(tmpdir(), "seashail-e2e-config-"));

    const bin = new URL("../../../target/debug/seashail", import.meta.url)
      .pathname;
    expect(existsSync(bin)).toBe(true);

    const p = spawn(bin, ["mcp"], {
      cwd: new URL("../../../", import.meta.url).pathname,
      stdio: ["pipe", "pipe", "pipe"],
      env: {
        ...process.env,
        RUST_LOG: "warn",
        SEASHAIL_DATA_DIR: dataDir,
        SEASHAIL_CONFIG_DIR: configDir,
      },
    });

    const outBuf = { text: "" };
    await rpcCall(p, outBuf, 1, "initialize", {});

    writeJsonLine(p, {
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: { name: "create_wallet", arguments: { name: "snap" } },
    });
    await driveElicitationWithHandler(p, outBuf, 2, defaultElicitationHandler);

    // Lending positions via native RPC (Aave + Compound). These should be safe read-only calls.
    const lendAave = await retry(
      "get_lending_positions(aave)",
      async () => {
        const r = await rpcCall(p, outBuf, 3, "tools/call", {
          name: "get_lending_positions",
          arguments: { wallet: "snap", chain: "ethereum", protocol: "aave" },
        });
        return mustToolOk(r, "get_lending_positions(aave)");
      },
      { tries: 4, delayMs: 900 }
    );
    expect(lendAave.source).toBe("rpc");

    const lendComp = await retry(
      "get_lending_positions(compound)",
      async () => {
        const r = await rpcCall(p, outBuf, 4, "tools/call", {
          name: "get_lending_positions",
          arguments: {
            wallet: "snap",
            chain: "ethereum",
            protocol: "compound",
          },
        });
        return mustToolOk(r, "get_lending_positions(compound)");
      },
      { tries: 4, delayMs: 900 }
    );
    expect(lendComp.source).toBe("rpc");

    const pred = await retry(
      "get_prediction_positions",
      async () => {
        const r = await rpcCall(p, outBuf, 5, "tools/call", {
          name: "get_prediction_positions",
          arguments: { wallet: "snap", chain: "polygon" },
        });
        return mustToolOk(r, "get_prediction_positions");
      },
      { tries: 4, delayMs: 900 }
    );
    expect(pred.source).toBe("polymarket_data_api");

    const port = await retry(
      "get_portfolio(include_health)",
      async () => {
        const r = await rpcCall(p, outBuf, 6, "tools/call", {
          name: "get_portfolio",
          arguments: { wallets: ["snap"], include_health: true },
        });
        return mustToolOk(r, "get_portfolio(include_health)");
      },
      { tries: 4, delayMs: 900 }
    );

    expect(typeof port.health).toBe("object");
    expect(Array.isArray(port.health.snapshots)).toBe(true);

    const surfaces = new Set<string>(
      extractSurfaces(port.health.snapshots as any[])
    );
    expect(surfaces.has("lending")).toBe(true);
    expect(surfaces.has("prediction")).toBe(true);

    p.stdin?.end();
    await new Response(p.stderr).text();
  }, 180_000);
});

describe("seashail MCP stdio (local fixtures)", () => {
  test("portfolio snapshots: include_history persists and returns P&L deltas (local solana validator)", async () => {
    const dataDir = mkdtempSync(join(tmpdir(), "seashail-e2e-data-"));
    const configDir = mkdtempSync(join(tmpdir(), "seashail-e2e-config-"));
    const ledgerDir = mkdtempSync(join(tmpdir(), "seashail-e2e-sol-ledger-"));

    const ports = await pickFreeSolanaPorts();
    const { rpcPort, gossipPort, faucetPort } = ports;
    const validator = spawn(
      "solana-test-validator",
      [
        "--reset",
        "--quiet",
        "--bind-address",
        "127.0.0.1",
        "--gossip-host",
        "127.0.0.1",
        "--gossip-port",
        String(gossipPort),
        "--rpc-port",
        String(rpcPort),
        "--faucet-port",
        String(faucetPort),
        "--ledger",
        ledgerDir,
      ],
      { stdio: ["ignore", "pipe", "pipe"] }
    );

    const valOut = { stdout: { text: "" }, stderr: { text: "" } };
    tailPipe(validator.stdout, valOut.stdout);
    tailPipe(validator.stderr, valOut.stderr);

    try {
      const solUrl = `http://127.0.0.1:${rpcPort}`;
      await waitForSolana(solUrl);

      const bin = new URL("../../../target/debug/seashail", import.meta.url)
        .pathname;
      expect(existsSync(bin)).toBe(true);

      const p = spawn(bin, ["mcp"], {
        cwd: new URL("../../../", import.meta.url).pathname,
        stdio: ["pipe", "pipe", "pipe"],
        env: {
          ...process.env,
          RUST_LOG: "warn",
          SEASHAIL_DATA_DIR: dataDir,
          SEASHAIL_CONFIG_DIR: configDir,
        },
      });

      const outBuf = { text: "" };
      await rpcCall(p, outBuf, 1, "initialize", {});

      writeJsonLine(p, {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: { name: "create_wallet", arguments: { name: "hist" } },
      });
      await driveElicitationWithHandler(
        p,
        outBuf,
        2,
        defaultElicitationHandler
      );

      await rpcCall(p, outBuf, 3, "tools/call", {
        name: "configure_rpc",
        arguments: { chain: "solana", url: solUrl },
      });

      const infoRes = await rpcCall(p, outBuf, 4, "tools/call", {
        name: "get_wallet_info",
        arguments: { wallet: "hist" },
      });
      const infoObj = mustToolOk(infoRes, "get_wallet_info") as any;
      const idx = infoObj.active_account as number;
      const solAddr = infoObj.addresses?.solana?.[idx] as string;
      expect(typeof solAddr).toBe("string");

      const a1 = await rpcCall(p, outBuf, 5, "tools/call", {
        name: "request_airdrop",
        arguments: {
          wallet: "hist",
          chain: "solana",
          amount: "1",
          amount_units: "ui",
        },
      });
      expect(parseToolPayload(a1).isError).toBe(false);
      await waitForSolanaBalance(solUrl, solAddr, 1);
      const bal1 = await getSolanaBalanceConfirmed(solUrl, solAddr);

      const p1 = await rpcCall(p, outBuf, 6, "tools/call", {
        name: "get_portfolio",
        arguments: {
          chains: ["solana"],
          include_history: true,
          history_limit: 10,
        },
      });
      const obj1 = mustToolOk(p1, "get_portfolio #1") as any;
      expect(typeof obj1.total_usd).toBe("number");
      expect(Array.isArray(obj1.history?.snapshots)).toBe(true);
      expect(obj1.history.snapshots.length).toBeGreaterThanOrEqual(1);

      await rpcCall(p, outBuf, 7, "tools/call", {
        name: "update_policy",
        arguments: { wallet: "hist", policy: { send_allow_any: true } },
      });

      const to = randomBytes(32);
      const bs58Mod = await import("bs58");
      const toB58 = bs58Mod.default.encode(to);
      const send = await rpcCall(p, outBuf, 8, "tools/call", {
        name: "send_transaction",
        arguments: {
          wallet: "hist",
          account_index: idx,
          chain: "solana",
          to: toB58,
          token: "native",
          amount: "0.05",
          amount_units: "ui",
        },
      });
      expect(parseToolPayload(send).isError).toBe(false);

      await waitForSolanaBalanceBelow(solUrl, solAddr, bal1);

      const p2 = await rpcCall(p, outBuf, 9, "tools/call", {
        name: "get_portfolio",
        arguments: {
          chains: ["solana"],
          include_history: true,
          history_limit: 10,
        },
      });
      const obj2 = mustToolOk(p2, "get_portfolio #2") as any;
      expect(obj2.history.snapshots.length).toBeGreaterThanOrEqual(2);
      expect(typeof obj2.pnl?.delta_since_prev_snapshot_usd).toBe("number");
      expect(obj2.pnl.delta_since_prev_snapshot_usd).not.toBe(0);

      const a = await rpcCall(p, outBuf, 10, "tools/call", {
        name: "get_portfolio_analytics",
        arguments: {
          limit: 50,
          snapshot_scope: { chains: ["solana"] },
        },
      });
      const aObj = mustToolOk(a, "get_portfolio_analytics") as any;
      expect(typeof aObj.snapshot_pnl?.delta_since_prev_snapshot_usd).toBe(
        "number"
      );

      p.stdin?.end();
      await new Response(p.stderr).text();
    } finally {
      validator.kill("SIGKILL");
    }
  }, 180_000);

  test("import_wallet (solana private key) + airdrop + send (local validator)", async () => {
    const dataDir = mkdtempSync(join(tmpdir(), "seashail-e2e-data-"));
    const configDir = mkdtempSync(join(tmpdir(), "seashail-e2e-config-"));
    const ledgerDir = mkdtempSync(join(tmpdir(), "seashail-e2e-sol-ledger-"));

    const ports = await pickFreeSolanaPorts();
    const { rpcPort, gossipPort, faucetPort } = ports;
    const validator = spawn(
      "solana-test-validator",
      [
        "--reset",
        "--quiet",
        "--bind-address",
        "127.0.0.1",
        "--gossip-host",
        "127.0.0.1",
        "--gossip-port",
        String(gossipPort),
        "--rpc-port",
        String(rpcPort),
        "--faucet-port",
        String(faucetPort),
        "--ledger",
        ledgerDir,
      ],
      { stdio: ["ignore", "pipe", "pipe"] }
    );

    try {
      const solUrl = `http://127.0.0.1:${rpcPort}`;
      await waitForSolana(solUrl);

      const { Keypair } = await import("@solana/web3.js");
      const bs58Mod2 = await import("bs58");
      const bs58 = bs58Mod2.default;
      const kp = Keypair.generate();
      const secretB58 = bs58.encode(Buffer.from(kp.secretKey));

      const bin = new URL("../../../target/debug/seashail", import.meta.url)
        .pathname;
      expect(existsSync(bin)).toBe(true);

      const p = spawn(bin, ["mcp"], {
        cwd: new URL("../../../", import.meta.url).pathname,
        stdio: ["pipe", "pipe", "pipe"],
        env: {
          ...process.env,
          RUST_LOG: "warn",
          SEASHAIL_DATA_DIR: dataDir,
          SEASHAIL_CONFIG_DIR: configDir,
        },
      });

      const outBuf = { text: "" };
      await rpcCall(p, outBuf, 1, "initialize", {});

      writeJsonLine(p, {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: {
          name: "import_wallet",
          arguments: {
            name: "sol-import",
            kind: "private_key",
            private_key_chain: "solana",
          },
        },
      });
      await driveElicitationWithHandler(
        p,
        outBuf,
        2,
        elicitationHandlerWithSecret(secretB58)
      );

      await rpcCall(p, outBuf, 3, "tools/call", {
        name: "update_policy",
        arguments: {
          policy: {
            auto_approve_usd: 100_000,
            confirm_up_to_usd: 100_000,
            hard_block_over_usd: 100_000,
            max_usd_per_tx: 100_000,
            max_usd_per_day: 1_000_000,
            max_slippage_bps: 5000,
            deny_unknown_usd_value: false,
            enable_send: true,
            send_allow_any: true,
          },
        },
      });

      await rpcCall(p, outBuf, 4, "tools/call", {
        name: "configure_rpc",
        arguments: { chain: "solana", url: solUrl },
      });

      const addr = kp.publicKey.toBase58();
      const air = await rpcCall(p, outBuf, 5, "tools/call", {
        name: "request_airdrop",
        arguments: {
          wallet: "sol-import",
          address: addr,
          amount: "1",
          amount_units: "ui",
        },
      });
      expect(parseToolPayload(air).isError).toBe(false);
      await waitForSolanaBalance(solUrl, addr, 500_000_000);

      const to = bs58.encode(randomBytes(32));
      const sendRes = await rpcCall(p, outBuf, 6, "tools/call", {
        name: "send_transaction",
        arguments: {
          wallet: "sol-import",
          chain: "solana",
          to,
          token: "native",
          amount: "0.01",
          amount_units: "ui",
        },
      });
      expect(parseToolPayload(sendRes).isError).toBe(false);

      p.stdin?.end();
      await new Response(p.stderr).text();
    } finally {
      validator.kill("SIGKILL");
    }
  }, 180_000);
});

function handleHealthMockFetch(
  req: Request,
  state: { lastKaminoPath: string }
): Response {
  const u = new URL(req.url);
  if (u.pathname.endsWith("/positions")) {
    return Response.json([{ market_id: "m1", size: "1" }]);
  }
  if (u.pathname.includes("/kamino-market/")) {
    state.lastKaminoPath = u.pathname;
    return Response.json([{ obligation_id: "o1", health_factor: "9.9" }]);
  }
  return new Response("not found", { status: 404 });
}

async function handleBtcMockFetch(
  req: Request,
  state: { lastBroadcastBody: string }
): Promise<Response> {
  const u = new URL(req.url);
  const parts = u.pathname.split("/").filter(Boolean);
  if (req.method === "GET" && parts[0] === "address" && parts.length === 2) {
    return Response.json({
      chain_stats: { funded_txo_sum: 100_000, spent_txo_sum: 0 },
      mempool_stats: { funded_txo_sum: 0, spent_txo_sum: 0 },
    });
  }
  if (
    req.method === "GET" &&
    parts[0] === "address" &&
    parts.length === 3 &&
    parts[2] === "utxo"
  ) {
    return Response.json([
      {
        txid: "00".repeat(32),
        vout: 0,
        value: 50_000,
      },
    ]);
  }
  if (req.method === "GET" && u.pathname === "/fee-estimates") {
    return Response.json({ "5": 1 });
  }
  if (req.method === "POST" && u.pathname === "/tx") {
    state.lastBroadcastBody = await req.text();
    return new Response("11".repeat(32), { status: 200 });
  }
  return new Response("not found", { status: 404 });
}

describe("seashail MCP stdio (offline mocks/fixtures)", () => {
  test("pump.fun read tools can run deterministically via fixture (no RPC)", async () => {
    const dataDir = mkdtempSync(join(tmpdir(), "seashail-e2e-data-"));
    const configDir = mkdtempSync(join(tmpdir(), "seashail-e2e-config-"));

    const bs58Mod = await import("bs58");
    const bs58 = bs58Mod.default;
    const mint = bs58.encode(randomBytes(32));
    const bondingCurve = bs58.encode(randomBytes(32));

    const fixture = JSON.stringify({
      items: [
        {
          mint,
          bonding_curve: bondingCurve,
          curve: {
            complete: false,
            creator: bs58.encode(randomBytes(32)),
            virtual_sol_reserves: "1",
            virtual_token_reserves: "2",
            real_sol_reserves: "3",
            real_token_reserves: "4",
            token_total_supply: "5",
            discriminator: "0",
          },
        },
      ],
    });

    const bin = new URL("../../../target/debug/seashail", import.meta.url)
      .pathname;
    expect(existsSync(bin)).toBe(true);

    const p = spawn(bin, ["mcp"], {
      cwd: new URL("../../../", import.meta.url).pathname,
      stdio: ["pipe", "pipe", "pipe"],
      env: {
        ...process.env,
        RUST_LOG: "warn",
        SEASHAIL_DATA_DIR: dataDir,
        SEASHAIL_CONFIG_DIR: configDir,
        SEASHAIL_PUMPFUN_DISCOVERY_FIXTURE_JSON: fixture,
      },
    });

    const outBuf = { text: "" };
    await rpcCall(p, outBuf, 1, "initialize", {});

    writeJsonLine(p, {
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: { name: "create_wallet", arguments: { name: "pf-fixture" } },
    });
    await driveElicitationWithHandler(p, outBuf, 2, defaultElicitationHandler);

    const listRes = await rpcCall(p, outBuf, 3, "tools/call", {
      name: "pumpfun_list_new_coins",
      arguments: { limit: 1 },
    });
    const listObj = mustToolOk(listRes, "pumpfun_list_new_coins") as any;
    expect(listObj.source).toBe("rpc");
    expect(listObj.fixture).toBe(true);
    expect(listObj.items?.[0]?.mint).toBe(mint);

    const infoRes = await rpcCall(p, outBuf, 4, "tools/call", {
      name: "pumpfun_get_coin_info",
      arguments: { mint },
    });
    const infoObj = mustToolOk(infoRes, "pumpfun_get_coin_info") as any;
    expect(infoObj.source).toBe("rpc");
    expect(infoObj.fixture).toBe(true);
    expect(infoObj.mint).toBe(mint);
    expect(infoObj.bonding_curve).toBe(bondingCurve);

    p.stdin?.end();
    await new Response(p.stderr).text();
  }, 60_000);

  test("health snapshots persist and surface via get_portfolio(include_health) using mocked upstreams", async () => {
    const dataDir = mkdtempSync(join(tmpdir(), "seashail-e2e-data-"));
    const configDir = mkdtempSync(join(tmpdir(), "seashail-e2e-config-"));

    const healthState = { lastKaminoPath: "" };
    const server = Bun.serve({
      port: 0,
      fetch(req) {
        return handleHealthMockFetch(req, healthState);
      },
    });
    const base = `http://127.0.0.1:${server.port}`;

    const bin = new URL("../../../target/debug/seashail", import.meta.url)
      .pathname;
    expect(existsSync(bin)).toBe(true);

    const p = spawn(bin, ["mcp"], {
      cwd: new URL("../../../", import.meta.url).pathname,
      stdio: ["pipe", "pipe", "pipe"],
      env: {
        ...process.env,
        RUST_LOG: "warn",
        SEASHAIL_DATA_DIR: dataDir,
        SEASHAIL_CONFIG_DIR: configDir,
        SEASHAIL_POLYMARKET_DATA_BASE_URL: `${base}/polymarket`,
        SEASHAIL_KAMINO_API_BASE_URL: base,
        SEASHAIL_KAMINO_DEFAULT_LEND_MARKET: "TEST_MARKET",
      },
    });

    const outBuf = { text: "" };
    await rpcCall(p, outBuf, 1, "initialize", {});

    writeJsonLine(p, {
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: { name: "create_wallet", arguments: { name: "snap-offline" } },
    });
    await driveElicitationWithHandler(p, outBuf, 2, defaultElicitationHandler);

    const lend = await rpcCall(p, outBuf, 3, "tools/call", {
      name: "get_lending_positions",
      arguments: {
        wallet: "snap-offline",
        chain: "solana",
        protocol: "kamino",
      },
    });
    const lendObj = mustToolOk(lend, "get_lending_positions(kamino)") as any;
    expect(lendObj.source).toBe("kamino_api");
    expect(
      healthState.lastKaminoPath.includes("/kamino-market/TEST_MARKET/")
    ).toBe(true);

    const pred = await rpcCall(p, outBuf, 4, "tools/call", {
      name: "get_prediction_positions",
      arguments: { wallet: "snap-offline", chain: "polygon" },
    });
    const predObj = mustToolOk(pred, "get_prediction_positions") as any;
    expect(predObj.source).toBe("polymarket_data_api");

    const port = await rpcCall(p, outBuf, 5, "tools/call", {
      name: "get_portfolio",
      arguments: { wallets: ["snap-offline"], include_health: true },
    });
    const portObj = mustToolOk(port, "get_portfolio(include_health)") as any;
    const surfaces = new Set<string>(
      extractSurfaces(portObj.health.snapshots as any[])
    );
    expect(surfaces.has("lending")).toBe(true);
    expect(surfaces.has("prediction")).toBe(true);

    p.stdin?.end();
    await new Response(p.stderr).text();
    server.stop(true);
  }, 90_000);

  test("bitcoin send_transaction works end-to-end with mocked Blockstream API", async () => {
    const dataDir = mkdtempSync(join(tmpdir(), "seashail-e2e-data-"));
    const configDir = mkdtempSync(join(tmpdir(), "seashail-e2e-config-"));

    const btcState = { lastBroadcastBody: "" };
    const btc = Bun.serve({
      port: 0,
      fetch(req) {
        return handleBtcMockFetch(req, btcState);
      },
    });
    const btcBase = `http://127.0.0.1:${btc.port}`;

    const bin = new URL("../../../target/debug/seashail", import.meta.url)
      .pathname;
    expect(existsSync(bin)).toBe(true);

    const p = spawn(bin, ["mcp"], {
      cwd: new URL("../../../", import.meta.url).pathname,
      stdio: ["pipe", "pipe", "pipe"],
      env: {
        ...process.env,
        RUST_LOG: "warn",
        SEASHAIL_DATA_DIR: dataDir,
        SEASHAIL_CONFIG_DIR: configDir,
        SEASHAIL_NETWORK_MODE: "testnet",
        SEASHAIL_BITCOIN_API_BASE_URL_TESTNET: btcBase,
      },
    });

    const outBuf = { text: "" };
    await rpcCall(p, outBuf, 1, "initialize", {});

    writeJsonLine(p, {
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: { name: "create_wallet", arguments: { name: "btc" } },
    });
    await driveElicitationWithHandler(p, outBuf, 2, defaultElicitationHandler);

    await rpcCall(p, outBuf, 3, "tools/call", {
      name: "update_policy",
      arguments: {
        wallet: "btc",
        policy: {
          auto_approve_usd: 100_000,
          confirm_up_to_usd: 100_000,
          hard_block_over_usd: 100_000,
          max_usd_per_tx: 100_000,
          max_usd_per_day: 1_000_000,
          deny_unknown_usd_value: false,
          enable_send: true,
          send_allow_any: true,
        },
      },
    });

    const infoRes = await rpcCall(p, outBuf, 4, "tools/call", {
      name: "get_wallet_info",
      arguments: { wallet: "btc" },
    });
    const infoObj = mustToolOk(infoRes, "get_wallet_info") as any;
    expect(Array.isArray(infoObj.addresses?.bitcoin_testnet)).toBe(true);

    const sendRes = await rpcCall(p, outBuf, 5, "tools/call", {
      name: "send_transaction",
      arguments: {
        wallet: "btc",
        chain: "bitcoin",
        to: "tb1qcr8te4kr609gcawutmrza0j4xv80jy8zmfp6l0",
        token: "native",
        amount: "1000",
        amount_units: "base",
      },
    });
    const sendObj = mustToolOk(sendRes, "send_transaction(bitcoin)") as any;
    expect(sendObj.chain).toBe("bitcoin");
    expect(typeof sendObj.txid).toBe("string");
    expect(sendObj.txid.length).toBeGreaterThan(0);
    expect(btcState.lastBroadcastBody.startsWith("02")).toBe(true); // raw tx hex

    p.stdin?.end();
    await new Response(p.stderr).text();
    btc.stop(true);
  }, 90_000);
});

describe("seashail MCP stdio (defi tx envelope: policy + allowlist + fail-closed)", () => {
  const envTest = HAVE_ANVIL ? test : test.skip;

  envTest(
    "layerzero bridge_tokens succeeds under default allowlisting; get_bridge_status uses adapter",
    async () => {
      const dataDir = mkdtempSync(join(tmpdir(), "seashail-e2e-data-"));
      const configDir = mkdtempSync(join(tmpdir(), "seashail-e2e-config-"));

      const port = await pickFreePort();
      const anvil = spawn(
        "anvil",
        ["--silent", "--port", String(port), "--chain-id", "11155111"],
        { stdio: ["ignore", "pipe", "pipe"] }
      );
      const anvilUrl = `http://127.0.0.1:${port}`;
      await waitForAnvil(anvilUrl);

      const LAYERZERO_ENDPOINT_V2 =
        "0x1a44076050125825900e736c501f859c50fE728c";
      // mock adapter: tx-envelope + bridge status
      const adapter = Bun.serve({
        port: 0,
        async fetch(req) {
          const u = new URL(req.url);
          if (req.method === "POST" && u.pathname === "/tx-envelope") {
            const body = (await req.json()) as any;
            if (
              body?.marketplace === "layerzero" &&
              body?.op === "bridge_tokens"
            ) {
              return Response.json({
                to: LAYERZERO_ENDPOINT_V2,
                data: "0x",
                value_wei: "0",
                usd_value: 1,
              });
            }
            return new Response("bad request", { status: 400 });
          }
          if (req.method === "GET" && u.pathname === "/bridge/status") {
            return Response.json({ phase: "completed" });
          }
          return new Response("not found", { status: 404 });
        },
      });
      const adapterBase = `http://127.0.0.1:${adapter.port}`;

      const bin = new URL("../../../target/debug/seashail", import.meta.url)
        .pathname;
      expect(existsSync(bin)).toBe(true);

      const p = spawn(bin, ["mcp"], {
        cwd: new URL("../../../", import.meta.url).pathname,
        stdio: ["pipe", "pipe", "pipe"],
        env: {
          ...process.env,
          RUST_LOG: "warn",
          SEASHAIL_DATA_DIR: dataDir,
          SEASHAIL_CONFIG_DIR: configDir,
          SEASHAIL_NETWORK_MODE: "testnet",
          SEASHAIL_DEFI_ADAPTER_BASE_URL: adapterBase,
        },
      });

      const outBuf = { text: "" };
      await rpcCall(p, outBuf, 1, "initialize", {});

      writeJsonLine(p, {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: { name: "create_wallet", arguments: { name: "lz" } },
      });
      await driveElicitationWithHandler(
        p,
        outBuf,
        2,
        defaultElicitationHandler
      );

      await rpcCall(p, outBuf, 3, "tools/call", {
        name: "configure_rpc",
        arguments: { chain: "sepolia", url: anvilUrl },
      });

      const infoRes = await rpcCall(p, outBuf, 4, "tools/call", {
        name: "get_wallet_info",
        arguments: { wallet: "lz" },
      });
      const infoObj = mustToolOk(infoRes, "get_wallet_info") as any;
      const idx = infoObj.active_account as number;
      const evmAddr = infoObj.addresses?.evm?.[idx] as string;
      expect(typeof evmAddr).toBe("string");

      // Fund wallet for gas.
      await anvilRpc(anvilUrl, "anvil_setBalance", [
        evmAddr,
        "0x3635C9ADC5DEA00000", // 1000 ETH
      ]);

      await rpcCall(p, outBuf, 5, "tools/call", {
        name: "update_policy",
        arguments: {
          wallet: "lz",
          policy: {
            auto_approve_usd: 100_000,
            confirm_up_to_usd: 100_000,
            hard_block_over_usd: 100_000,
            max_usd_per_tx: 100_000,
            max_usd_per_day: 1_000_000,
            deny_unknown_usd_value: false,
            enable_send: true,
            send_allow_any: true,
            enable_bridge: true,
            max_usd_per_bridge_tx: 100_000,
          },
        },
      });

      const bridgeRes = await rpcCall(p, outBuf, 6, "tools/call", {
        name: "bridge_tokens",
        arguments: {
          wallet: "lz",
          chain: "sepolia",
          bridge_provider: "layerzero",
          asset: { token: "usdc", amount: "1", amount_units: "ui" },
        },
      });
      const bridgeObj = mustToolOk(
        bridgeRes,
        "bridge_tokens(layerzero)"
      ) as any;
      expect(bridgeObj.chain).toBe("sepolia");
      expect(typeof bridgeObj.txid).toBe("string");

      const statusRes = await rpcCall(p, outBuf, 7, "tools/call", {
        name: "get_bridge_status",
        arguments: {
          bridge_provider: "layerzero",
          bridge_id: bridgeObj.txid,
        },
      });
      const statusObj = mustToolOk(
        statusRes,
        "get_bridge_status(layerzero)"
      ) as any;
      expect(statusObj.source).toBe("defi_adapter");
      expect(statusObj.status?.phase).toBe("completed");

      p.stdin?.end();
      await new Response(p.stderr).text();
      adapter.stop(true);
      anvil.kill("SIGKILL");
    },
    120_000
  );

  envTest(
    "defi tx envelope: policy disabled + contract allowlist rejection + simulation fail-closed",
    async () => {
      const dataDir = mkdtempSync(join(tmpdir(), "seashail-e2e-data-"));
      const configDir = mkdtempSync(join(tmpdir(), "seashail-e2e-config-"));

      const port = await pickFreePort();
      const anvil = spawn(
        "anvil",
        ["--silent", "--port", String(port), "--chain-id", "11155111"],
        { stdio: ["ignore", "pipe", "pipe"] }
      );
      const anvilUrl = `http://127.0.0.1:${port}`;
      await waitForAnvil(anvilUrl);

      // Deploy a minimal reverting contract for deterministic simulation failure.
      const deployTx = (await anvilRpc(anvilUrl, "eth_sendTransaction", [
        {
          from: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
          data: "0x6005600c60003960056000f360006000fd",
        },
      ])) as string;
      const receipt = await waitForReceipt(anvilUrl, deployTx);
      const revertAddr = receipt.contractAddress as string;
      expect(typeof revertAddr).toBe("string");

      let mode: "ok" | "unallowlisted" | "revert" = "ok";
      const adapter = Bun.serve({
        port: 0,
        fetch(req) {
          const u = new URL(req.url);
          if (req.method === "POST" && u.pathname === "/tx-envelope") {
            if (mode === "unallowlisted") {
              return Response.json({
                to: "0x000000000000000000000000000000000000BEEF",
                data: "0x",
                value_wei: "0",
                usd_value: 1,
              });
            }
            if (mode === "revert") {
              return Response.json({
                to: revertAddr,
                data: "0x",
                value_wei: "0",
                usd_value: 1,
              });
            }
            return Response.json({
              to: "0x1a44076050125825900e736c501f859c50fE728c",
              data: "0x",
              value_wei: "0",
              usd_value: 1,
            });
          }
          return new Response("not found", { status: 404 });
        },
      });
      const adapterBase = `http://127.0.0.1:${adapter.port}`;

      const bin = new URL("../../../target/debug/seashail", import.meta.url)
        .pathname;
      expect(existsSync(bin)).toBe(true);

      const p = spawn(bin, ["mcp"], {
        cwd: new URL("../../../", import.meta.url).pathname,
        stdio: ["pipe", "pipe", "pipe"],
        env: {
          ...process.env,
          RUST_LOG: "warn",
          SEASHAIL_DATA_DIR: dataDir,
          SEASHAIL_CONFIG_DIR: configDir,
          SEASHAIL_NETWORK_MODE: "testnet",
          SEASHAIL_DEFI_ADAPTER_BASE_URL: adapterBase,
        },
      });

      const outBuf = { text: "" };
      await rpcCall(p, outBuf, 1, "initialize", {});

      writeJsonLine(p, {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: { name: "create_wallet", arguments: { name: "neg" } },
      });
      await driveElicitationWithHandler(
        p,
        outBuf,
        2,
        defaultElicitationHandler
      );

      await rpcCall(p, outBuf, 3, "tools/call", {
        name: "configure_rpc",
        arguments: { chain: "sepolia", url: anvilUrl },
      });

      const infoRes = await rpcCall(p, outBuf, 4, "tools/call", {
        name: "get_wallet_info",
        arguments: { wallet: "neg" },
      });
      const infoObj = mustToolOk(infoRes, "get_wallet_info") as any;
      const idx = infoObj.active_account as number;
      const evmAddr = infoObj.addresses?.evm?.[idx] as string;
      await anvilRpc(anvilUrl, "anvil_setBalance", [
        evmAddr,
        "0x3635C9ADC5DEA00000",
      ]);

      // 1) policy disabled
      await rpcCall(p, outBuf, 5, "tools/call", {
        name: "update_policy",
        arguments: {
          wallet: "neg",
          policy: {
            auto_approve_usd: 100_000,
            confirm_up_to_usd: 100_000,
            hard_block_over_usd: 100_000,
            max_usd_per_tx: 100_000,
            max_usd_per_day: 1_000_000,
            deny_unknown_usd_value: false,
            enable_bridge: false,
            enable_send: true,
            send_allow_any: true,
          },
        },
      });
      const dis = await rpcCall(p, outBuf, 6, "tools/call", {
        name: "bridge_tokens",
        arguments: {
          wallet: "neg",
          chain: "sepolia",
          bridge_provider: "layerzero",
          asset: { token: "usdc", amount: "1", amount_units: "ui" },
        },
      });
      const disP = parseToolPayload(dis);
      expect(disP.isError).toBe(true);
      expect(String(disP.payload?.code ?? disP.payload)).toContain(
        "policy_bridge_disabled"
      );

      // 2) contract not allowlisted
      await rpcCall(p, outBuf, 7, "tools/call", {
        name: "update_policy",
        arguments: {
          wallet: "neg",
          policy: {
            enable_bridge: true,
            contract_allow_any: false,
            contract_allowlist: [],
          },
        },
      });
      mode = "unallowlisted";
      const na = await rpcCall(p, outBuf, 8, "tools/call", {
        name: "bridge_tokens",
        arguments: {
          wallet: "neg",
          chain: "sepolia",
          bridge_provider: "layerzero",
          asset: { token: "usdc", amount: "1", amount_units: "ui" },
        },
      });
      const naP = parseToolPayload(na);
      expect(naP.isError).toBe(true);
      expect(String(naP.payload?.code ?? naP.payload)).toContain(
        "policy_contract_not_allowlisted"
      );

      // 3) simulation fail-closed (allow any contract so we test simulation path deterministically)
      await rpcCall(p, outBuf, 9, "tools/call", {
        name: "update_policy",
        arguments: {
          wallet: "neg",
          policy: {
            contract_allow_any: true,
          },
        },
      });
      mode = "revert";

      const beforeHist = await rpcCall(p, outBuf, 10, "tools/call", {
        name: "get_transaction_history",
        arguments: { wallet: "neg", limit: 50 },
      });
      const beforeObj = mustToolOk(
        beforeHist,
        "get_transaction_history(before)"
      ) as any;
      const beforeCount = (beforeObj?.items?.length ?? 0) as number;

      const sim = await rpcCall(p, outBuf, 11, "tools/call", {
        name: "bridge_tokens",
        arguments: {
          wallet: "neg",
          chain: "sepolia",
          bridge_provider: "layerzero",
          asset: { token: "usdc", amount: "1", amount_units: "ui" },
        },
      });
      const simP = parseToolPayload(sim);
      expect(simP.isError).toBe(true);
      expect(String(simP.payload?.code ?? simP.payload)).toContain(
        "simulation_failed"
      );

      const afterHist = await rpcCall(p, outBuf, 12, "tools/call", {
        name: "get_transaction_history",
        arguments: { wallet: "neg", limit: 50 },
      });
      const afterObj = mustToolOk(
        afterHist,
        "get_transaction_history(after)"
      ) as any;
      const afterCount = (afterObj?.items?.length ?? 0) as number;
      expect(afterCount).toBe(beforeCount); // fail-closed: no history write, no broadcast

      p.stdin?.end();
      await new Response(p.stderr).text();
      adapter.stop(true);
      anvil.kill("SIGKILL");
    },
    150_000
  );
});

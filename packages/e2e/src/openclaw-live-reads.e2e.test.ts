import { beforeAll, describe, expect, test } from "bun:test";
import assert from "node:assert/strict";
import { execFileSync, execSync } from "node:child_process";
import { existsSync } from "node:fs";

import { OpenClawHarness } from "./tests/openclaw-harness";

function hasCmd(cmd: string, args: string[] = ["--version"]): boolean {
  try {
    execFileSync(cmd, args, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

async function retry<T>(
  label: string,
  fn: () => Promise<T>,
  opts?: { tries?: number; delayMs?: number }
): Promise<T> {
  const tries = opts?.tries ?? 4;
  const delayMs = opts?.delayMs ?? 400;
  let lastErr: unknown = null;
  for (let i = 0; i < tries; i += 1) {
    try {
      return await fn();
    } catch (error) {
      lastErr = error;
      await new Promise<void>((resolve) => {
        setTimeout(resolve, delayMs * (i + 1));
      });
    }
  }
  throw new Error(`${label} failed after ${tries} tries: ${String(lastErr)}`);
}

const HAVE_OPENCLAW = hasCmd("openclaw", ["--version"]);
const SEASHAIL_E2E_LIVE = process.env["SEASHAIL_E2E_LIVE"] === "1";
const ocTest = HAVE_OPENCLAW && SEASHAIL_E2E_LIVE ? test : test.skip;

// Explicit public endpoints (avoid env var coupling; keep tests self-contained).
// These match Seashail's default config, but we still call `configure_rpc` so the
// OpenClaw integration is tested against known-good production reads.
const SOLANA_RPC_MAINNET = "https://api.mainnet-beta.solana.com";
const SOLANA_RPC_MAINNET_FALLBACKS = [
  "https://solana-rpc.publicnode.com",
  "https://rpc.ankr.com/solana",
  "https://solana.drpc.org",
];

const ETHEREUM_RPC_MAINNET = "https://eth.llamarpc.com";
const ETHEREUM_RPC_MAINNET_FALLBACKS = [
  "https://ethereum-rpc.publicnode.com",
  "https://rpc.ankr.com/eth",
  "https://cloudflare-eth.com",
];

async function configurePublicRpcMainnet(h: OpenClawHarness): Promise<void> {
  // Configure both Solana + Ethereum so failures are attributable to upstream
  // endpoints (not local defaults/env).
  await retry("configure_rpc(solana)", async () => {
    const r = await h.invokeSeashailTool("configure_rpc", {
      chain: "solana",
      url: SOLANA_RPC_MAINNET,
      fallback_urls: SOLANA_RPC_MAINNET_FALLBACKS,
      mode: "mainnet",
    });
    if (!r.ok) {
      throw new Error(r.rawText);
    }
    return r;
  });

  await retry("configure_rpc(ethereum)", async () => {
    const r = await h.invokeSeashailTool("configure_rpc", {
      chain: "ethereum",
      url: ETHEREUM_RPC_MAINNET,
      fallback_urls: ETHEREUM_RPC_MAINNET_FALLBACKS,
    });
    if (!r.ok) {
      throw new Error(r.rawText);
    }
    return r;
  });
}

describe("OpenClaw live read smoke (real RPC/HTTP endpoints; no mocks)", () => {
  let seashailBin = "";
  let pluginPath = "";

  beforeAll(() => {
    execSync("cargo build -p seashail", {
      cwd: new URL("../../../", import.meta.url).pathname,
      timeout: 180_000,
      stdio: "pipe",
    });
    seashailBin = new URL("../../../target/debug/seashail", import.meta.url)
      .pathname;
    expect(existsSync(seashailBin)).toBe(true);
    pluginPath = new URL("../../openclaw-seashail-plugin", import.meta.url)
      .pathname;
  }, 240_000);

  ocTest(
    "get_balance hits real Solana/EVM RPC + Bitcoin HTTP API",
    async () => {
      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "mainnet",
      });

      try {
        await configurePublicRpcMainnet(h);

        // Query each chain explicitly so we can attribute failures.
        const sol = await retry("get_balance(solana)", async () => {
          const r = await h.invokeSeashailTool("get_balance", {
            chain: "solana",
          });
          if (!r.ok) {
            throw new Error(r.rawText);
          }
          const b0 = (r.payload as any)?.balances?.[0];
          if (b0?.error) {
            throw new Error(String(b0.error));
          }
          return r;
        });
        expect(sol.ok).toBe(true);
        const solBalances = (sol.payload as any)?.balances;
        expect(Array.isArray(solBalances)).toBe(true);
        expect(solBalances[0]?.chain).toBe("solana");

        const evm = await retry("get_balance(ethereum)", async () => {
          const r = await h.invokeSeashailTool("get_balance", {
            chain: "ethereum",
          });
          if (!r.ok) {
            throw new Error(r.rawText);
          }
          const b0 = (r.payload as any)?.balances?.[0];
          if (b0?.error) {
            throw new Error(String(b0.error));
          }
          return r;
        });
        expect(evm.ok).toBe(true);
        const evmBalances = (evm.payload as any)?.balances;
        expect(Array.isArray(evmBalances)).toBe(true);
        expect(evmBalances[0]?.chain).toBe("ethereum");

        const btc = await retry("get_balance(bitcoin)", async () => {
          const r = await h.invokeSeashailTool("get_balance", {
            chain: "bitcoin",
          });
          if (!r.ok) {
            throw new Error(r.rawText);
          }
          const b0 = (r.payload as any)?.balances?.[0];
          if (b0?.error) {
            throw new Error(String(b0.error));
          }
          return r;
        });
        expect(btc.ok).toBe(true);
        const btcBalances = (btc.payload as any)?.balances;
        expect(Array.isArray(btcBalances)).toBe(true);
        expect(btcBalances[0]?.chain).toBe("bitcoin");
      } finally {
        await h.cleanup();
      }
    },
    180_000
  );

  ocTest(
    "inspect_token hits real EVM RPC (USDC decimals/symbol)",
    async () => {
      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "mainnet",
      });
      try {
        await configurePublicRpcMainnet(h);

        // Ethereum mainnet USDC.
        const usdc = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
        const inspected = await retry("inspect_token(usdc)", async () => {
          const r = await h.invokeSeashailTool("inspect_token", {
            chain: "ethereum",
            token: usdc,
          });
          if (!r.ok) {
            throw new Error(String(r.rawText ?? "inspect_token failed"));
          }
          return r;
        });
        expect(inspected.ok).toBe(true);
        const p = inspected.payload as any;
        expect(p?.kind).toBe("erc20");
        expect(p?.decimals).toBe(6);
        expect(String(p?.symbol ?? "")).toMatch(/USDC/i);
      } finally {
        await h.cleanup();
      }
    },
    180_000
  );

  ocTest(
    "default wallet exists immediately (no LLM chat required)",
    async () => {
      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "mainnet",
      });
      try {
        await configurePublicRpcMainnet(h);

        const listed = await h.invokeSeashailTool("list_wallets", {});
        expect(listed.ok).toBe(true);
        const wallets = (listed.payload as any)?.wallets;
        assert.ok(Array.isArray(wallets), "wallets must be an array");
        expect(wallets.some((w: any) => w?.name === "default")).toBe(true);
      } finally {
        await h.cleanup();
      }
    },
    120_000
  );
});

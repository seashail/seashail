import { beforeAll, afterAll, describe, expect, test } from "bun:test";
import assert from "node:assert/strict";
import { execSync } from "node:child_process";
import { existsSync } from "node:fs";

import {
  makeDefaultElicitationHandler,
  makeTestPassphrase,
} from "./tests/elicitation";
import { OpenClawHarness, type ElicitReply } from "./tests/openclaw-harness";

const PASSPHRASE = makeTestPassphrase();
const handleElicitation = makeDefaultElicitationHandler(PASSPHRASE) as (
  message: string
) => ElicitReply;

function assertNotPassphrasePrompt(message: string) {
  const isPassphrase =
    message.includes("Set a Seashail passphrase") ||
    message.includes("Enter your Seashail passphrase");
  if (isPassphrase) {
    throw new Error(`unexpected passphrase elicitation: ${message}`);
  }
}

describe("seashail OpenClaw plugin (end-to-end)", () => {
  let h: OpenClawHarness | null = null;

  beforeAll(async () => {
    // Ensure the debug binary exists and matches the current workspace sources.
    execSync("cargo build -p seashail", {
      cwd: new URL("../../../", import.meta.url).pathname,
      timeout: 120_000,
      stdio: "pipe",
    });

    const seashailBin = new URL(
      "../../../target/debug/seashail",
      import.meta.url
    ).pathname;
    expect(existsSync(seashailBin)).toBe(true);

    const pluginPath = new URL(
      "../../openclaw-seashail-plugin",
      import.meta.url
    ).pathname;

    h = await OpenClawHarness.create({
      seashailBinPath: seashailBin,
      pluginPath,
      network: "testnet",
      // Intentionally do NOT set passphrase env var; exercise resume flow.
    });
  }, 180_000);

  afterAll(async () => {
    if (h) {
      await h.cleanup();
      h = null;
    }
  });

  test("prefixed tools are invokable (get_capabilities)", async () => {
    assert.ok(h, "missing harness");
    const res = await h.invokeSeashailTool(
      "get_capabilities",
      {},
      handleElicitation
    );
    expect(res.ok).toBe(true);
    expect(typeof res.payload).toBe("object");
  });

  test("prefixed tools are invokable (get_testnet_faucet_links)", async () => {
    assert.ok(h, "missing harness");
    const res = await h.invokeSeashailTool(
      "get_testnet_faucet_links",
      {
        chain: "sepolia",
        address: "0x000000000000000000000000000000000000dEaD",
      },
      handleElicitation
    );
    expect(res.ok).toBe(true);
    const payload = res.payload as any;
    expect(typeof payload).toBe("object");
    expect(Array.isArray(payload?.faucets)).toBe(true);
    expect(payload.faucets.length).toBeGreaterThan(0);
  });

  test("unprefixed tool names are not exposed", async () => {
    assert.ok(h, "missing harness");
    await expect(
      h.invokeTool("get_testnet_faucet_links", {})
    ).rejects.toThrow();
  });

  test("resume token flow works (create_wallet)", async () => {
    assert.ok(h, "missing harness");
    const res = await h.invokeSeashailTool(
      "create_wallet",
      { name: `openclaw-e2e-${Date.now()}` },
      handleElicitation
    );
    expect(res.ok).toBe(true);
  }, 60_000);

  test("wallet tools work (list_wallets / get_wallet_info / set_active_wallet)", async () => {
    assert.ok(h, "missing harness");

    const name = `openclaw-e2e-${Date.now()}`;
    const created = await h.invokeSeashailTool(
      "create_wallet",
      { name },
      handleElicitation
    );
    expect(created.ok).toBe(true);

    const listed = await h.invokeSeashailTool(
      "list_wallets",
      {},
      handleElicitation
    );
    expect(listed.ok).toBe(true);
    const wallets = (listed.payload as any)?.wallets;
    expect(Array.isArray(wallets)).toBe(true);
    expect(wallets.some((w: any) => w?.name === name)).toBe(true);

    const info = await h.invokeSeashailTool(
      "get_wallet_info",
      { wallet: name },
      handleElicitation
    );
    expect(info.ok).toBe(true);

    const active = await h.invokeSeashailTool(
      "set_active_wallet",
      { wallet: name, account_index: 0 },
      handleElicitation
    );
    expect(active.ok).toBe(true);
  }, 90_000);

  test("passphrase env var auto-answers passphrase prompts (no needs_approval for passphrase)", async () => {
    // Separate harness to avoid cross-test state.
    execSync("cargo build -p seashail", {
      cwd: new URL("../../../", import.meta.url).pathname,
      timeout: 120_000,
      stdio: "pipe",
    });

    const seashailBin = new URL(
      "../../../target/debug/seashail",
      import.meta.url
    ).pathname;
    expect(existsSync(seashailBin)).toBe(true);
    const pluginPath = new URL(
      "../../openclaw-seashail-plugin",
      import.meta.url
    ).pathname;

    const h2 = await OpenClawHarness.create({
      seashailBinPath: seashailBin,
      pluginPath,
      network: "testnet",
      passphrase: PASSPHRASE,
    });
    try {
      const handler = (message: string): ElicitReply => {
        assertNotPassphrasePrompt(message);
        return handleElicitation(message);
      };
      const res = await h2.invokeSeashailTool(
        "create_wallet",
        { name: `openclaw-e2e-${Date.now()}` },
        handler
      );
      expect(res.ok).toBe(true);
    } finally {
      await h2.cleanup();
    }
  }, 90_000);

  test("gateway logs do not contain passphrase material", () => {
    assert.ok(h, "missing harness");
    const logs = h.getGatewayLogs();
    expect(logs.stdout.includes(PASSPHRASE)).toBe(false);
    expect(logs.stderr.includes(PASSPHRASE)).toBe(false);
  });
});

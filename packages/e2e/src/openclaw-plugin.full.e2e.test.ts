import { beforeAll, describe, expect, test } from "bun:test";
import { execFileSync, execSync, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import net from "node:net";
import { join } from "node:path";

import {
  extractShareFromMessage,
  makeDefaultElicitationHandler,
  makeTestPassphrase,
} from "./tests/elicitation";
import { OpenClawHarness, type ElicitReply } from "./tests/openclaw-harness";
import { listSeashailMcpToolNames } from "./tests/seashail-mcp";

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
  const delayMs = opts?.delayMs ?? 500;
  let lastErr: unknown = null;
  for (let i = 0; i < tries; i += 1) {
    try {
      return await fn();
    } catch (error) {
      lastErr = error;
      await new Promise<void>((r) => {
        setTimeout(r, delayMs * (i + 1));
      });
    }
  }
  throw new Error(`${label} failed after ${tries} tries: ${String(lastErr)}`);
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
    await new Promise<void>((r) => {
      setTimeout(r, 200);
    });
  }
  throw new Error("anvil did not start");
}

const PASSPHRASE = makeTestPassphrase();
const defaultElicitationHandler = makeDefaultElicitationHandler(PASSPHRASE) as (
  message: string
) => ElicitReply;

function declinePolicyConfirmOnly(message: string): ElicitReply {
  if (
    message.startsWith("Seashail policy requires confirmation.") ||
    message.startsWith("Seashail requires confirmation.")
  ) {
    return { action: "decline" };
  }
  return defaultElicitationHandler(message);
}

function declinePassphraseOnly(message: string): ElicitReply {
  const isPw =
    message.includes("Set a Seashail passphrase") ||
    message.includes("Enter your Seashail passphrase");
  if (isPw) {
    return { action: "decline" };
  }
  return defaultElicitationHandler(message);
}

function assertNotPassphrasePrompt(message: string) {
  const isPw =
    message.includes("Set a Seashail passphrase") ||
    message.includes("Enter your Seashail passphrase");
  if (isPw) {
    throw new Error(`unexpected passphrase elicitation: ${message}`);
  }
}

const HAVE_OPENCLAW = hasCmd("openclaw", ["--version"]);
const HAVE_ANVIL = hasCmd("anvil", ["--version"]);

// Bun has no dynamic skip-if helper; pick the right function once.
const ocTest = HAVE_OPENCLAW ? test : test.skip;

describe("seashail OpenClaw plugin (full end-to-end coverage)", () => {
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
    "install/uninstall path works via `seashail openclaw install` (tools invokable)",
    async () => {
      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
      });
      try {
        const caps = await h.invokeSeashailTool(
          "get_capabilities",
          {},
          defaultElicitationHandler
        );
        expect(caps.ok).toBe(true);

        // Seamless onboarding: default wallet exists and deposit addresses are available immediately.
        const listed = await h.invokeSeashailTool(
          "list_wallets",
          {},
          defaultElicitationHandler
        );
        expect(listed.ok).toBe(true);
        const wallets = (listed.payload as any)?.wallets;
        expect(Array.isArray(wallets)).toBe(true);
        const def = wallets.find((w: any) => w?.name === "default");
        expect(typeof def).toBe("object");

        const solDep = await h.invokeSeashailTool(
          "get_deposit_info",
          { chain: "solana" },
          defaultElicitationHandler
        );
        expect(solDep.ok).toBe(true);
        expect(typeof (solDep.payload as any)?.address).toBe("string");

        const evmDep = await h.invokeSeashailTool(
          "get_deposit_info",
          { chain: "ethereum" },
          defaultElicitationHandler
        );
        expect(evmDep.ok).toBe(true);
        expect(String((evmDep.payload as any)?.address)).toMatch(
          /^0x[0-9a-fA-F]{40}$/
        );

        const btcDep = await h.invokeSeashailTool(
          "get_deposit_info",
          { chain: "bitcoin" },
          defaultElicitationHandler
        );
        expect(btcDep.ok).toBe(true);
        expect(typeof (btcDep.payload as any)?.address).toBe("string");

        await expect(h.invokeTool("get_capabilities", {})).rejects.toThrow();
      } finally {
        await h.cleanup();
      }
    },
    120_000
  );

  ocTest(
    "all registered Seashail tool names are reachable via /tools/invoke (not 404)",
    async () => {
      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
      });
      try {
        // Use OpenClaw's own plugin registry as the source of truth for what it thinks is registered.
        const raw = execFileSync(
          "openclaw",
          ["plugins", "info", "seashail", "--json"],
          {
            env: h.env,
            stdio: "pipe",
          }
        ).toString("utf8");
        const info = JSON.parse(raw) as any;
        const toolNames = Array.isArray(info?.toolNames)
          ? (info.toolNames as string[])
          : [];
        expect(toolNames.length).toBeGreaterThan(10);
        expect(
          toolNames.every(
            (t) => typeof t === "string" && t.startsWith("seashail_")
          )
        ).toBe(true);

        // Must include resume (elicitation continuation) and a few representative tools.
        const mustHave = [
          "seashail_resume",
          "seashail_get_capabilities",
          "seashail_list_wallets",
          "seashail_get_deposit_info",
          "seashail_get_balance",
          "seashail_send_transaction",
          "seashail_swap_tokens",
        ];
        for (const name of mustHave) {
          expect(toolNames.includes(name)).toBe(true);
        }

        // Spot-check that the gateway recognizes a few tools (not 404).
        // Do NOT iterate every tool name here; some tools perform network calls and this would
        // add minutes + flakiness. Registration coverage is enforced by `toolNames` above.
        const toCheck = [
          "seashail_get_capabilities",
          "seashail_list_wallets",
          "seashail_get_deposit_info",
        ];
        for (const tool of toCheck) {
          const resp = await fetch(
            `http://127.0.0.1:${h.gatewayPort}/tools/invoke`,
            {
              method: "POST",
              headers: {
                Authorization: `Bearer ${h.gatewayToken}`,
                "content-type": "application/json",
              },
              body: JSON.stringify({ tool, args: {} }),
            }
          );
          expect(resp.status).toBe(200);
          const text = await resp.text();
          const parsed = JSON.parse(text) as any;
          expect(parsed?.ok).toBe(true);
        }

        // A nonexistent tool should be rejected.
        await expect(
          h.invokeTool("seashail__definitely_not_a_tool__", {})
        ).rejects.toThrow();
      } finally {
        await h.cleanup();
      }
    },
    180_000
  );

  ocTest(
    "OpenClaw plugin registers the full Seashail MCP tool surface (no missing tools)",
    async () => {
      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
      });
      try {
        const raw = execFileSync(
          "openclaw",
          ["plugins", "info", "seashail", "--json"],
          { env: h.env, stdio: "pipe" }
        ).toString("utf8");
        const info = JSON.parse(raw) as any;
        const pluginTools = Array.isArray(info?.toolNames)
          ? (info.toolNames as string[])
          : [];

        const mcpTools = await listSeashailMcpToolNames({
          seashailBinPath: seashailBin,
          env: {
            ...h.env,
            SEASHAIL_CONFIG_DIR: h.seashailConfigDir,
            SEASHAIL_DATA_DIR: h.seashailDataDir,
          },
        });

        const pluginUnprefixed = new Set(
          pluginTools
            .filter((t) => t !== "seashail_resume")
            .map((t) => (t.startsWith("seashail_") ? t.slice(9) : t))
        );
        const mcpSet = new Set(mcpTools);

        const missing: string[] = [];
        for (const t of mcpSet) {
          if (!pluginUnprefixed.has(t)) {
            missing.push(t);
          }
        }

        const extra: string[] = [];
        for (const t of pluginUnprefixed) {
          if (t === "resume") {
            continue;
          }
          if (!mcpSet.has(t)) {
            extra.push(t);
          }
        }

        expect(missing).toEqual([]);
        expect(extra).toEqual([]);
      } finally {
        await h.cleanup();
      }
    },
    180_000
  );

  ocTest(
    "passphrase session expiry requires re-unlock (decline => user_declined)",
    async () => {
      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
        // Disable install-time wallet onboarding so the OpenClaw plugin does not configure
        // an auto-unlock passphrase file (we want to exercise interactive unlock behavior).
        onboardWallet: false,
        // Seed the config before any Seashail processes start so passphrase session expiry is
        // deterministic and not dependent on runtime config reload behavior.
        seedSeashailConfigToml: `passphrase_session_seconds = 1\n`,
      });
      try {
        const wname = `oc-exp-${Date.now()}`;
        const created = await h.invokeSeashailTool(
          "create_wallet",
          { name: wname },
          defaultElicitationHandler
        );
        expect(created.ok).toBe(true);

        const warg = { wallet: wname };

        const r0 = await h.invokeSeashailTool(
          "rotate_shares",
          warg,
          defaultElicitationHandler
        );
        expect(r0.ok).toBe(true);

        await new Promise<void>((r) => {
          setTimeout(r, 1200);
        });

        const r1 = await h.invokeSeashailTool(
          "rotate_shares",
          warg,
          declinePassphraseOnly
        );
        expect(r1.ok).toBe(false);
        const payload = r1.payload as any;
        expect(payload?.code).toBe("user_declined");
      } finally {
        await h.cleanup();
      }
    },
    120_000
  );

  const ocAnvilTest = HAVE_OPENCLAW && HAVE_ANVIL ? test : test.skip;

  ocAnvilTest(
    "policy hard cap blocks send_transaction (no approval path)",
    async () => {
      const port = await pickFreePort();
      const anvil = spawn(
        "anvil",
        ["--chain-id", "1", "--port", String(port)],
        { stdio: ["ignore", "pipe", "pipe"] }
      );

      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
      });
      try {
        const anvilUrl = `http://127.0.0.1:${port}`;
        await waitForAnvil(anvilUrl);

        const wname = `oc-cap-${Date.now()}`;
        const created = await h.invokeSeashailTool(
          "create_wallet",
          { name: wname },
          defaultElicitationHandler
        );
        expect(created.ok).toBe(true);

        const policy = {
          auto_approve_usd: 0,
          confirm_up_to_usd: 1,
          hard_block_over_usd: 1,
          max_usd_per_tx: 100_000,
          max_usd_per_day: 1_000_000,
          max_slippage_bps: 5000,
          deny_unknown_usd_value: true,
          require_user_confirm_for_remote_tx: true,
          enable_send: true,
          enable_swap: true,
          send_allow_any: true,
          send_allowlist: [],
          contract_allow_any: true,
          contract_allowlist: [],
        };
        const upd = await h.invokeSeashailTool(
          "update_policy",
          { policy },
          defaultElicitationHandler
        );
        expect(upd.ok).toBe(true);

        const cfgRpc = await h.invokeSeashailTool(
          "configure_rpc",
          { chain: "ethereum", url: anvilUrl },
          defaultElicitationHandler
        );
        expect(cfgRpc.ok).toBe(true);

        // Use real production reads for pricing; retry to avoid transient upstream flakes.
        await retry("get_token_price(ethereum,native)", async () => {
          const px = await h.invokeSeashailTool(
            "get_token_price",
            { chain: "ethereum", token: "native" },
            defaultElicitationHandler
          );
          if (!px.ok) {
            throw new Error(String((px.payload as any)?.code ?? px.rawText));
          }
          return px;
        });

        const info = await h.invokeSeashailTool(
          "get_wallet_info",
          { wallet: wname },
          defaultElicitationHandler
        );
        expect(info.ok).toBe(true);
        const from = (info.payload as any)?.addresses?.evm?.[0];
        expect(typeof from).toBe("string");
        await anvilRpc(anvilUrl, "anvil_setBalance", [
          from,
          "0x8AC7230489E80000",
        ]);

        const to = "0x000000000000000000000000000000000000dEaD";
        const send = await retry(
          "send_transaction(policy hard cap)",
          async () => {
            const r = await h.invokeSeashailTool(
              "send_transaction",
              {
                wallet: wname,
                chain: "ethereum",
                to,
                token: "native",
                amount: "0.002",
                amount_units: "ui",
              },
              defaultElicitationHandler
            );
            if (r.ok) {
              throw new Error("expected policy rejection, got ok=true");
            }
            const code = String((r.payload as any)?.code ?? "");
            if (
              code === "policy_usd_value_unknown" ||
              code === "price_unavailable"
            ) {
              throw new Error(`transient price unavailable: ${code}`);
            }
            return r;
          }
        );
        expect(send.ok).toBe(false);
        expect((send.payload as any)?.code).toBe("policy_hard_block");
      } finally {
        await h.cleanup();
        anvil.kill("SIGKILL");
      }
    },
    180_000
  );

  ocAnvilTest(
    "policy confirmation decline blocks send_transaction (+ no send in history)",
    async () => {
      const port = await pickFreePort();
      const anvil = spawn(
        "anvil",
        ["--chain-id", "1", "--port", String(port)],
        { stdio: ["ignore", "pipe", "pipe"] }
      );

      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
      });
      try {
        const anvilUrl = `http://127.0.0.1:${port}`;
        await waitForAnvil(anvilUrl);

        const wname = `oc-decline-${Date.now()}`;
        const created = await h.invokeSeashailTool(
          "create_wallet",
          { name: wname },
          defaultElicitationHandler
        );
        expect(created.ok).toBe(true);

        const policy = {
          auto_approve_usd: 0,
          confirm_up_to_usd: 100_000,
          hard_block_over_usd: 100_000,
          max_usd_per_tx: 100_000,
          max_usd_per_day: 1_000_000,
          max_slippage_bps: 5000,
          enable_send: true,
          enable_swap: true,
          send_allow_any: true,
          send_allowlist: [],
          contract_allow_any: false,
          contract_allowlist: [],
        };
        const upd = await h.invokeSeashailTool(
          "update_policy",
          { wallet: wname, policy },
          defaultElicitationHandler
        );
        expect(upd.ok).toBe(true);

        const cfgRpc = await h.invokeSeashailTool(
          "configure_rpc",
          { chain: "ethereum", url: anvilUrl },
          defaultElicitationHandler
        );
        expect(cfgRpc.ok).toBe(true);

        await retry("get_token_price(ethereum,native)", async () => {
          const px = await h.invokeSeashailTool(
            "get_token_price",
            { chain: "ethereum", token: "native" },
            defaultElicitationHandler
          );
          if (!px.ok) {
            throw new Error(String((px.payload as any)?.code ?? px.rawText));
          }
          return px;
        });

        const info = await h.invokeSeashailTool(
          "get_wallet_info",
          { wallet: wname },
          defaultElicitationHandler
        );
        expect(info.ok).toBe(true);
        const from = (info.payload as any)?.addresses?.evm?.[0];
        expect(typeof from).toBe("string");
        await anvilRpc(anvilUrl, "anvil_setBalance", [
          from,
          "0x8AC7230489E80000",
        ]);

        const to = "0x000000000000000000000000000000000000dEaD";
        const send = await retry(
          "send_transaction(policy decline)",
          async () => {
            const r = await h.invokeSeashailTool(
              "send_transaction",
              {
                wallet: wname,
                chain: "ethereum",
                to,
                token: "native",
                amount: "0.001",
                amount_units: "ui",
              },
              declinePolicyConfirmOnly
            );
            if (r.ok) {
              throw new Error(
                "expected policy confirmation decline, got ok=true"
              );
            }
            const code = String((r.payload as any)?.code ?? "");
            if (
              code === "policy_usd_value_unknown" ||
              code === "price_unavailable"
            ) {
              throw new Error(`transient price unavailable: ${code}`);
            }
            return r;
          }
        );
        expect(send.ok).toBe(false);
        expect((send.payload as any)?.code).toBe("user_declined");

        const hist = await h.invokeSeashailTool(
          "get_transaction_history",
          { wallet: wname, limit: 200 },
          defaultElicitationHandler
        );
        expect(hist.ok).toBe(true);
        const items = (hist.payload as any)?.items;
        expect(Array.isArray(items)).toBe(true);
        const types = new Set(items.map((i: any) => String(i?.type ?? "")));
        expect(types.has("wallet_created")).toBe(true);
        expect(types.has("send")).toBe(false);
        expect(types.has("approve")).toBe(false);
      } finally {
        await h.cleanup();
        anvil.kill("SIGKILL");
      }
    },
    240_000
  );

  ocTest(
    "multiple gateways share one daemon state (passphrase session shared)",
    async () => {
      const sharedBase = process.platform === "win32" ? undefined : "/tmp";
      const sharedConfigDir = join(
        sharedBase ?? "/tmp",
        `seashail-oc-shared-config-${Date.now()}`
      );
      const sharedDataDir = join(
        sharedBase ?? "/tmp",
        `seashail-oc-shared-data-${Date.now()}`
      );

      let h1: OpenClawHarness | null = null;
      let h2: OpenClawHarness | null = null;
      try {
        h1 = await OpenClawHarness.create({
          seashailBinPath: seashailBin,
          pluginPath,
          network: "testnet",
          seashailConfigDir: sharedConfigDir,
          seashailDataDir: sharedDataDir,
        });
        const wname = `oc-shared-${Date.now()}`;
        const created = await h1.invokeSeashailTool(
          "create_wallet",
          { name: wname },
          defaultElicitationHandler
        );
        expect(created.ok).toBe(true);

        h2 = await OpenClawHarness.create({
          seashailBinPath: seashailBin,
          pluginPath,
          network: "testnet",
          seashailConfigDir: sharedConfigDir,
          seashailDataDir: sharedDataDir,
        });
        const handler = (msg: string): ElicitReply => {
          assertNotPassphrasePrompt(msg);
          return defaultElicitationHandler(msg);
        };
        const rot = await h2.invokeSeashailTool(
          "rotate_shares",
          { wallet: wname },
          handler
        );
        expect(rot.ok).toBe(true);
      } finally {
        await h2?.cleanup();
        await h1?.cleanup();
      }
    },
    180_000
  );

  ocTest(
    "write lock contention returns keystore_busy (no hang) [standalone]",
    async () => {
      const sharedBase = process.platform === "win32" ? undefined : "/tmp";
      const sharedConfigDir = join(
        sharedBase ?? "/tmp",
        `seashail-oc-lock-config-${Date.now()}`
      );
      const sharedDataDir = join(
        sharedBase ?? "/tmp",
        `seashail-oc-lock-data-${Date.now()}`
      );

      const h1 = await OpenClawHarness.create({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
        seashailConfigDir: sharedConfigDir,
        seashailDataDir: sharedDataDir,
        standalone: true,
      });
      const h2 = await OpenClawHarness.create({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
        seashailConfigDir: sharedConfigDir,
        seashailDataDir: sharedDataDir,
        standalone: true,
      });

      try {
        // Start a write and intentionally do not resume the passphrase prompt, to hold the lock.
        const res = (await h1.invokeTool(`${h1.seashailPrefix}create_wallet`, {
          name: `lock-${Date.now()}`,
        })) as any;
        const details =
          (res && typeof res === "object" ? (res as any).details : null) ?? {};
        expect(details?.status).toBe("needs_approval");

        const blocked = await h2.invokeSeashailTool(
          "create_wallet",
          { name: `blocked-${Date.now()}` },
          defaultElicitationHandler
        );
        expect(blocked.ok).toBe(false);
        expect((blocked.payload as any)?.code).toBe("keystore_busy");
      } finally {
        await h2.cleanup();
        await h1.cleanup();
      }
    },
    120_000
  );

  ocTest(
    "no secrets in logs (passphrase, imported key, shamir share)",
    async () => {
      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
      });
      try {
        // Capture share3 from the wallet creation prompt (this is considered sensitive; never log it).
        let share3 = "";
        const handler = (message: string): ElicitReply => {
          if (message.includes("Offline backup share")) {
            share3 = extractShareFromMessage(message);
          }
          return defaultElicitationHandler(message);
        };

        const wname = `oc-logs-${Date.now()}`;
        const created = await h.invokeSeashailTool(
          "create_wallet",
          { name: wname },
          handler
        );
        expect(created.ok).toBe(true);
        expect(share3.length).toBeGreaterThan(10);

        // Import an EVM private key via elicitation (never via tool args).
        const priv = `0x${"11".repeat(32)}`;
        const importHandler = (message: string): ElicitReply => {
          if (message.includes("Confirm wallet import.")) {
            return { action: "accept", content: { confirm: true } };
          }
          // Best-effort: Seashail import prompt schemas vary by kind; reply with the private key under common keys.
          if (
            message.toLowerCase().includes("private key") ||
            message.toLowerCase().includes("import")
          ) {
            return {
              action: "accept",
              content: { private_key: priv, key: priv, secret: priv },
            };
          }
          return defaultElicitationHandler(message);
        };
        const imp = await h.invokeSeashailTool(
          "import_wallet",
          {
            name: `oc-import-${Date.now()}`,
            kind: "private_key",
            private_key_chain: "evm",
          },
          importHandler
        );
        expect(imp.ok).toBe(true);

        const gw = h.getGatewayLogs();
        const disk = h.getSeashailDiskLogs();
        const combined = [
          gw.stdout,
          gw.stderr,
          disk.seashailLogJsonl,
          disk.auditJsonl,
        ].join("\n");

        expect(combined.includes(PASSPHRASE)).toBe(false);
        expect(combined.includes(priv)).toBe(false);
        expect(combined.includes(share3)).toBe(false);
        expect(combined.includes("SHARE3_BASE64")).toBe(false);
      } finally {
        await h.cleanup();
      }
    },
    180_000
  );
});

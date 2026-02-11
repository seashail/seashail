import { beforeAll, describe, expect, test } from "bun:test";
import { execFileSync, execSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";

import {
  makeDefaultElicitationHandler,
  makeTestPassphrase,
} from "./tests/elicitation";
import { OpenClawHarness, type ElicitReply } from "./tests/openclaw-harness";

const MODEL_PROVIDER = "anthropic";
const DEFAULT_MODEL = "anthropic/claude-opus-4-6";

function hasCmd(cmd: string, args: string[] = ["--version"]): boolean {
  try {
    execFileSync(cmd, args, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function stripAnsiSgr(s: string): string {
  // Remove common SGR ANSI sequences: ESC [ ... m
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

function extractFirstJsonObject(out: string): any | null {
  const clean = stripAnsiSgr(out);
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

function parseOpenClawJson(out: string): any {
  const obj = extractFirstJsonObject(out);
  if (!obj || typeof obj !== "object") {
    throw new Error(
      `openclaw returned non-json output (first 200 chars): ${stripAnsiSgr(out).slice(0, 200)}`
    );
  }
  return obj;
}

function readOpenClawModelsStatus(env?: Record<string, string>): any {
  const raw = execFileSync("openclaw", ["models", "status", "--json"], {
    env,
    stdio: "pipe",
  }).toString("utf8");
  return parseOpenClawJson(raw);
}

function getAuthStorePathFromStatus(status: any): string {
  const p = status?.auth?.storePath;
  if (typeof p !== "string" || !p) {
    throw new Error("openclaw models status missing auth.storePath");
  }
  return p;
}

function configureAuthForHarness(h: OpenClawHarness) {
  const token = process.env["OPENCLAW_E2E_ANTHROPIC_TOKEN"] ?? "";
  if (token) {
    // Configure auth inside the isolated OpenClaw state dir (do not touch ~/.openclaw).
    execFileSync(
      "openclaw",
      [
        "models",
        "auth",
        "paste-token",
        "--provider",
        MODEL_PROVIDER,
        "--profile-id",
        `${MODEL_PROVIDER}:ci`,
        "--expires-in",
        "30d",
      ],
      {
        env: h.env,
        stdio: "pipe",
        input: token,
      }
    );
  } else {
    // Local dev fallback: reuse the user's already-configured OpenClaw auth store.
    // We copy the auth store into the isolated harness so the test can run without extra setup.
    const userStatus = readOpenClawModelsStatus(process.env as any);
    const userStorePath = getAuthStorePathFromStatus(userStatus);

    const harnessStatus = readOpenClawModelsStatus(h.env);
    const harnessStorePath = getAuthStorePathFromStatus(harnessStatus);
    mkdirSync(dirname(harnessStorePath), { recursive: true });

    copyFileSync(userStorePath, harnessStorePath);
  }

  // Ensure the harness has an explicit default model set.
  execFileSync("openclaw", ["models", "set", DEFAULT_MODEL], {
    env: h.env,
    stdio: "pipe",
  });

  // Sanity-check: model should appear as configured/available.
  const listedRaw = execFileSync("openclaw", ["models", "list", "--json"], {
    env: h.env,
    stdio: "pipe",
  }).toString("utf8");
  const listed = parseOpenClawJson(listedRaw);
  const count = typeof listed?.count === "number" ? listed.count : 0;
  if (count < 1) {
    throw new Error(
      "openclaw models list returned 0 models. Set OPENCLAW_E2E_ANTHROPIC_TOKEN to run agent chat tests in isolated mode."
    );
  }
}

const HAVE_OPENCLAW = hasCmd("openclaw", ["--version"]);

// Bun has no dynamic skip-if helper; pick the right function once.
const ocChatTest = HAVE_OPENCLAW ? test : test.skip;

describe("OpenClaw agent chat (real OpenClaw, no mocks)", () => {
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

  ocChatTest(
    "chat invokes Seashail tools via OpenClaw agent and returns correct deposit addresses",
    async () => {
      const passphrase = makeTestPassphrase();
      const handler = makeDefaultElicitationHandler(passphrase) as (
        message: string
      ) => ElicitReply;

      const h = await OpenClawHarness.createViaSeashailOpenclawInstall({
        seashailBinPath: seashailBin,
        pluginPath,
        network: "testnet",
      });

      try {
        configureAuthForHarness(h);

        // Ground truth: deposit addresses from direct tool calls.
        const solDep = await h.invokeSeashailTool(
          "get_deposit_info",
          { chain: "solana" },
          handler
        );
        expect(solDep.ok).toBe(true);
        const evmDep = await h.invokeSeashailTool(
          "get_deposit_info",
          { chain: "ethereum" },
          handler
        );
        expect(evmDep.ok).toBe(true);
        const btcDep = await h.invokeSeashailTool(
          "get_deposit_info",
          { chain: "bitcoin" },
          handler
        );
        expect(btcDep.ok).toBe(true);

        const wantSol = String((solDep.payload as any)?.address ?? "");
        const wantEvm = String((evmDep.payload as any)?.address ?? "");
        const wantBtc = String((btcDep.payload as any)?.address ?? "");
        expect(wantSol.length).toBeGreaterThan(10);
        expect(wantEvm).toMatch(/^0x[0-9a-fA-F]{40}$/);
        expect(wantBtc.length).toBeGreaterThan(10);

        const out = execFileSync(
          "openclaw",
          [
            "agent",
            "--session-id",
            `seashail-openclaw-chat-${Date.now()}`,
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
            "180",
            "--thinking",
            "off",
          ],
          { env: h.env, stdio: "pipe" }
        ).toString("utf8");

        const parsed = extractFirstJsonObject(out);
        expect(parsed && typeof parsed === "object").toBe(true);
        expect((parsed as any).status).toBe("ok");
        expect((parsed as any).result?.meta?.aborted).toBe(false);
        const textVal = (parsed as any).result?.payloads?.[0]?.text;
        expect(typeof textVal).toBe("string");
        expect(String(textVal).length).toBeGreaterThan(0);

        const jsonOut = extractFirstJsonObject(String(textVal));
        expect(jsonOut && typeof jsonOut === "object").toBe(true);

        expect(String((jsonOut as any).solana)).toBe(wantSol);
        expect(String((jsonOut as any).ethereum)).toBe(wantEvm);
        expect(String((jsonOut as any).bitcoin)).toBe(wantBtc);
      } finally {
        await h.cleanup();
      }
    },
    240_000
  );
});

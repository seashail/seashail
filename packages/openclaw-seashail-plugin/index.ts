import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";
import { randomBytes } from "node:crypto";

type JsonRpcId = string | number;

type JsonRpcRequest = {
  jsonrpc: "2.0";
  id?: JsonRpcId;
  method: string;
  params?: unknown;
};

type JsonRpcResponse = {
  jsonrpc: "2.0";
  id: JsonRpcId;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
};

type McpTool = {
  name: string;
  description?: string;
  inputSchema?: Record<string, unknown>;
};

type PluginConfig = {
  seashailPath?: unknown;
  network?: unknown;
  standalone?: unknown;
  toolPrefix?: unknown;
  prefix?: unknown;
  passphraseEnvVar?: unknown;
  env?: unknown;
};

type ResumeTokenState = {
  // Server->client request id for elicitation/create.
  elicitationId: JsonRpcId;
  // Promise for the original tools/call response.
  pendingToolCall: Promise<JsonRpcResponse>;
  // The tool name that triggered the prompt (for UX only).
  toolName: string;
  // Original MCP tool call id so we can disambiguate if needed.
  toolCallId: JsonRpcId;
  // Prompt payload.
  message: string;
  requestedSchema: unknown;
};

function asRecord(v: unknown): Record<string, unknown> | null {
  if (!v || typeof v !== "object" || Array.isArray(v)) return null;
  return v as Record<string, unknown>;
}

function isJsonObjectLine(s: string): boolean {
  const t = s.trim();
  return t.startsWith("{") && t.endsWith("}");
}

function safeJsonParse(s: string): unknown | undefined {
  try {
    return JSON.parse(s);
  } catch {
    return undefined;
  }
}

function resolveExecutablePath(seashailPathRaw: string | undefined) {
  const seashailPath = seashailPathRaw?.trim() || "seashail";

  // If config uses the default "seashail" (PATH), try common install locations
  // first. OpenClaw's gateway often runs as a user service where PATH does not
  // include ~/.cargo/bin.
  if (seashailPath === "seashail") {
    const home = process.env["HOME"] ?? "";
    const candidates =
      process.platform === "win32"
        ? []
        : [
            home ? path.join(home, ".cargo", "bin", "seashail") : "",
            "/usr/local/bin/seashail",
            "/opt/homebrew/bin/seashail",
          ].filter(Boolean);

    for (const c of candidates) {
      try {
        const stat = fs.statSync(c);
        if (!stat.isFile()) continue;
        if (process.platform !== "win32") {
          fs.accessSync(c, fs.constants.X_OK);
        }
        return c;
      } catch {
        // try next
      }
    }
  }

  // SECURITY:
  // Never allow arbitrary executables (e.g. /bin/bash). If the caller overrides
  // the path, it must still be the seashail binary (by name) and be absolute.
  if (seashailPath !== "seashail") {
    if (!path.isAbsolute(seashailPath)) {
      throw new Error("seashailPath must be an absolute path (or omit to use PATH)");
    }
    const base = path.basename(seashailPath).toLowerCase();
    const allowed =
      process.platform === "win32"
        ? ["seashail.exe", "seashail.cmd", "seashail.bat"]
        : ["seashail"];
    if (!allowed.includes(base)) {
      throw new Error("seashailPath must point to the seashail executable");
    }
    let stat: fs.Stats;
    try {
      stat = fs.statSync(seashailPath);
    } catch {
      throw new Error("seashailPath must exist");
    }
    if (!stat.isFile()) {
      throw new Error("seashailPath must point to a file");
    }
    if (process.platform !== "win32") {
      try {
        fs.accessSync(seashailPath, fs.constants.X_OK);
      } catch {
        throw new Error("seashailPath must be executable");
      }
    }
  }

  return seashailPath;
}

function randToken(): string {
  return randomBytes(16).toString("hex");
}

class SeashailMcpClient {
  private child: ReturnType<typeof spawn> | null = null;
  private rl: readline.Interface | null = null;
  private nextId = 1;
  private pending = new Map<JsonRpcId, { resolve: (r: JsonRpcResponse) => void; reject: (e: Error) => void }>();
  private stderrBuf = "";
  private starting: Promise<void> | null = null;
  private ready = false;

  private onElicitationCreate: ((req: { id: JsonRpcId; params: Record<string, unknown> }) => void) | null =
    null;

  setElicitationHandler(fn: ((req: { id: JsonRpcId; params: Record<string, unknown> }) => void) | null) {
    this.onElicitationCreate = fn;
  }

  async ensureStarted(config: {
    seashailPath: string;
    args: string[];
    env: Record<string, string | undefined>;
  }) {
    if (this.ready && this.child && this.child.exitCode === null) return;
    if (this.starting) {
      await this.starting;
      return;
    }

    this.starting = (async () => {
      this.stop();

      const child = spawn(config.seashailPath, config.args, {
        stdio: ["pipe", "pipe", "pipe"],
        env: config.env,
        windowsHide: true,
      });
      this.child = child;

      child.stdin?.setDefaultEncoding("utf8");
      child.stdout?.setEncoding("utf8");
      child.stderr?.setEncoding("utf8");
      // Drain stderr to avoid backpressure deadlocks. Keep a small ring buffer
      // for debugging when the subprocess exits unexpectedly.
      this.stderrBuf = "";
      child.stderr?.on("data", (d) => {
        const next = this.stderrBuf + String(d);
        this.stderrBuf = next.length > 20_000 ? next.slice(next.length - 20_000) : next;
      });

      const rl = readline.createInterface({ input: child.stdout! });
      this.rl = rl;

      rl.on("line", (line) => {
        const trimmed = line.trim();
        if (!trimmed) return;
        if (!isJsonObjectLine(trimmed)) return;

        const msg = safeJsonParse(trimmed);
        const rec = asRecord(msg);
        if (!rec) return;

        const method = typeof rec["method"] === "string" ? (rec["method"] as string) : "";
        const id = rec["id"] as JsonRpcId | undefined;

        // Server->client request (elicitation/create)
        if (method && id !== undefined) {
          if (method === "elicitation/create") {
            const params = asRecord(rec["params"]) ?? {};
            this.onElicitationCreate?.({ id, params });
            return;
          }
          // Unknown request type: reply method not found.
          this.write({
            jsonrpc: "2.0",
            id,
            error: { code: -32601, message: "method not found" },
          } satisfies JsonRpcResponse);
          return;
        }

        // Client response
        if (id === undefined) return;
        const entry = this.pending.get(id);
        if (!entry) return;

        this.pending.delete(id);
        entry.resolve(msg as JsonRpcResponse);
      });

      const onDead = (why: string) => {
        this.ready = false;
        const meta = `code=${child.exitCode ?? "?"} signal=${(child as any).signalCode ?? "?"}`;
        const tail = this.stderrBuf.trim() ? `\n\nstderr (tail):\n${this.stderrBuf.trim()}` : "";
        for (const [_id, p] of this.pending) {
          p.reject(new Error(`Seashail MCP subprocess died (${why}) (${meta})${tail}`));
        }
        this.pending.clear();
        this.child = null;
        this.rl?.close();
        this.rl = null;
      };

      child.once("exit", () => onDead("exit"));
      child.once("error", () => onDead("error"));

      // MCP init
      const initId = this.nextId++;
      const initResp = await this.request({
        jsonrpc: "2.0",
        id: initId,
        method: "initialize",
        params: {},
      });
      if (initResp.error) {
        throw new Error(`initialize failed: ${initResp.error.message}`);
      }
      this.ready = true;
    })();

    try {
      await this.starting;
    } finally {
      this.starting = null;
    }
  }

  stop() {
    this.ready = false;
    this.starting = null;
    try {
      this.rl?.close();
    } catch {}
    this.rl = null;

    if (this.child && this.child.exitCode === null) {
      try {
        this.child.kill("SIGKILL");
      } catch {}
    }
    this.child = null;

    for (const [_id, p] of this.pending) {
      p.reject(new Error("Seashail MCP client stopped"));
    }
    this.pending.clear();
  }

  private write(msg: unknown) {
    const child = this.child;
    if (!child?.stdin) throw new Error("Seashail MCP subprocess not running");
    child.stdin.write(`${JSON.stringify(msg)}\n`);
  }

  request(req: JsonRpcRequest): Promise<JsonRpcResponse> {
    const id = req.id;
    if (id === undefined) throw new Error("id required");

    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.write(req);
    });
  }

  async listTools(): Promise<McpTool[]> {
    const id = this.nextId++;
    const resp = await this.request({ jsonrpc: "2.0", id, method: "tools/list", params: {} });
    if (resp.error) throw new Error(resp.error.message);
    const rec = asRecord(resp.result);
    const tools = rec ? rec["tools"] : undefined;
    if (!Array.isArray(tools)) return [];
    return tools
      .map((t) => asRecord(t))
      .filter((t): t is Record<string, unknown> => !!t)
      .map((t) => ({
        name: typeof t["name"] === "string" ? (t["name"] as string) : "",
        description: typeof t["description"] === "string" ? (t["description"] as string) : undefined,
        inputSchema: asRecord(t["inputSchema"]) ?? { type: "object", properties: {} },
      }))
      .filter((t) => !!t.name);
  }

  async callTool(toolName: string, args: unknown): Promise<{ toolCallId: JsonRpcId; pending: Promise<JsonRpcResponse> }> {
    const id = this.nextId++;
    const pending = this.request({
      jsonrpc: "2.0",
      id,
      method: "tools/call",
      params: { name: toolName, arguments: args ?? {} },
    });
    return { toolCallId: id, pending };
  }

  async respondElicitation(reqId: JsonRpcId, result: unknown) {
    this.write({ jsonrpc: "2.0", id: reqId, result } satisfies JsonRpcResponse);
  }
}

type ToolSpec = { name: string; description?: string; inputSchema?: Record<string, unknown> };

function extractJsonBangObjects(raw: string): string[] {
  // Parse `json!({ ... })` blocks from the Rust tool schema registry.
  // We do a lightweight brace-matching scan so we can recover `inputSchema`
  // without spawning the Seashail binary at plugin-load time.
  const out: string[] = [];
  let i = 0;
  for (;;) {
    const idx = raw.indexOf("json!({", i);
    if (idx === -1) break;
    const start = raw.indexOf("{", idx);
    if (start === -1) break;

    let depth = 0;
    let inStr = false;
    let esc = false;
    let end = -1;
    for (let j = start; j < raw.length; j += 1) {
      const ch = raw[j] as string;
      if (inStr) {
        if (esc) {
          esc = false;
          continue;
        }
        if (ch === "\\") {
          esc = true;
          continue;
        }
        if (ch === '"') {
          inStr = false;
        }
        continue;
      }

      if (ch === '"') {
        inStr = true;
        continue;
      }
      if (ch === "{") {
        depth += 1;
        continue;
      }
      if (ch === "}") {
        depth -= 1;
        if (depth === 0) {
          end = j + 1;
          break;
        }
      }
    }

    if (end !== -1) {
      out.push(raw.slice(start, end));
      i = end;
    } else {
      // Unbalanced; stop scanning to avoid an infinite loop.
      break;
    }
  }
  return out;
}

function loadToolSpecsFromRepoSchema(): ToolSpec[] {
  // Best-effort: when developing from the monorepo, mirror the authoritative
  // Rust tool registry (`schema.rs`) so OpenClaw exposes all Seashail tools.
  //
  // In non-repo installs, this file won't exist; we fall back to a small fixed set.
  try {
    const schemaPath = new URL(
      "../../crates/seashail/src/rpc/mcp_server/tools/schema.rs",
      import.meta.url,
    ).pathname;
    const raw = fs.readFileSync(schemaPath, "utf8");
    const out: ToolSpec[] = [];
    const seen = new Set<string>();
    for (const objText of extractJsonBangObjects(raw)) {
      // `schema.rs` is Rust source. `json!({ ... })` blocks are JSON-like, but may include
      // Rust numeric literal separators (e.g. 10_000_000), which are invalid JSON.
      const normalized = objText.replace(/(\d)_(\d)/g, "$1$2");
      const v = safeJsonParse(normalized);
      const rec = asRecord(v);
      if (!rec) continue;
      const name = typeof rec["name"] === "string" ? (rec["name"] as string).trim() : "";
      if (!name || seen.has(name)) continue;
      const description = typeof rec["description"] === "string" ? (rec["description"] as string) : undefined;
      const inputSchema = asRecord(rec["inputSchema"]) ?? undefined;
      seen.add(name);
      out.push({ name, ...(description ? { description } : {}), ...(inputSchema ? { inputSchema } : {}) });
    }
    if (out.length > 0) return out;
  } catch {
    // ignore
  }

  // Minimal fallback (should not drift too much).
  return [
    { name: "get_capabilities", inputSchema: { type: "object", properties: {}, additionalProperties: false } },
    {
      name: "get_testnet_faucet_links",
      inputSchema: {
        type: "object",
        properties: { chain: { type: "string" }, address: { type: "string" } },
        required: ["chain"],
        additionalProperties: false,
      },
    },
    { name: "create_wallet", inputSchema: { type: "object", properties: { name: { type: "string" } }, required: ["name"] } },
    { name: "list_wallets", inputSchema: { type: "object", properties: {}, additionalProperties: false } },
    { name: "get_wallet_info", inputSchema: { type: "object", properties: { wallet: { type: "string" } }, additionalProperties: false } },
    {
      name: "set_active_wallet",
      inputSchema: {
        type: "object",
        properties: { wallet: { type: "string" }, account_index: { type: "integer", minimum: 0 } },
        required: ["wallet", "account_index"],
        additionalProperties: false,
      },
    },
  ];
}

export default function register(api: any) {
  const rawCfg = (api.pluginConfig ?? {}) as PluginConfig;
  const seashailPath = resolveExecutablePath(typeof rawCfg.seashailPath === "string" ? rawCfg.seashailPath : undefined);
  const network = typeof rawCfg.network === "string" && rawCfg.network === "testnet" ? "testnet" : "mainnet";
  const standalone = rawCfg.standalone === true;
  const toolPrefix = rawCfg.toolPrefix === true;
  const prefix = typeof rawCfg.prefix === "string" && rawCfg.prefix.trim() ? rawCfg.prefix.trim() : "seashail_";
  const passphraseEnvVar =
    typeof rawCfg.passphraseEnvVar === "string" && rawCfg.passphraseEnvVar.trim()
      ? rawCfg.passphraseEnvVar.trim()
      : "SEASHAIL_PASSPHRASE";
  const extraEnv = asRecord(rawCfg.env) ?? {};

  const args = (() => {
    const argv = ["mcp"];
    if (network === "testnet") argv.push("--network", "testnet");
    if (standalone) argv.push("--standalone");
    return argv;
  })();

  const env: Record<string, string | undefined> = { ...process.env };
  for (const [k, v] of Object.entries(extraEnv)) {
    if (typeof v === "string") env[k] = v;
  }

  const client = new SeashailMcpClient();
  const resumeTokens = new Map<string, ResumeTokenState>();

  // Server->client elicitation requests arrive out-of-band relative to the
  // `tools/call` response. We serialize tool calls, so a simple FIFO is enough,
  // but we must avoid polling (timer leaks) and support cancelation.
  let elicitationQueue: Array<{ id: JsonRpcId; params: Record<string, unknown> }> = [];
  const elicitationWaiters: Array<(req: { id: JsonRpcId; params: Record<string, unknown> }) => void> = [];
  client.setElicitationHandler((req) => {
    const next = elicitationWaiters.shift();
    if (next) {
      next(req);
      return;
    }
    elicitationQueue.push(req);
  });

  function waitForElicitation(): { promise: Promise<{ id: JsonRpcId; params: Record<string, unknown> }>; cancel: () => void } {
    const queued = elicitationQueue.shift();
    if (queued) {
      return { promise: Promise.resolve(queued), cancel: () => {} };
    }

    let active = true;
    let resolve: ((v: { id: JsonRpcId; params: Record<string, unknown> }) => void) | null = null;
    const promise = new Promise<{ id: JsonRpcId; params: Record<string, unknown> }>((res) => {
      resolve = res;
    });
    const waiter = (v: { id: JsonRpcId; params: Record<string, unknown> }) => {
      if (!active) return;
      active = false;
      resolve?.(v);
    };
    elicitationWaiters.push(waiter);
    const cancel = () => {
      if (!active) return;
      active = false;
      const i = elicitationWaiters.indexOf(waiter);
      if (i >= 0) {
        elicitationWaiters.splice(i, 1);
      }
    };
    return { promise, cancel };
  }

  // Serialize tool calls: Seashail uses interactive server->client prompts, so concurrency is messy.
  let queue = Promise.resolve();
  const enqueue = async <T>(fn: () => Promise<T>) => {
    const next = queue.then(fn, fn);
    queue = next.then(
      () => undefined,
      () => undefined,
    );
      return await next;
  };

  async function ensureClient() {
    await client.ensureStarted({
      seashailPath,
      args,
      env,
    });
  }

  function isPassphraseSchema(schema: unknown): boolean {
    const rec = asRecord(schema);
    const props = rec ? asRecord(rec["properties"]) : null;
    return !!(props && typeof props["passphrase"] === "object");
  }

  async function runToolUntilYield(toolName: string, params: unknown, pendingCall?: { toolCallId: JsonRpcId; pending: Promise<JsonRpcResponse> }) {
    await ensureClient();

    // Drop any stray queued elicitations from prior calls.
    elicitationQueue = [];
    const call = pendingCall ?? (await client.callTool(toolName, params));

    // Wait for either tool response or a server-side elicitation prompt.
    while (true) {
      const el = waitForElicitation();
      const winner = await Promise.race([
        call.pending.then((resp) => ({ kind: "response" as const, resp })),
        el.promise.then((req) => ({ kind: "elicit" as const, req })),
      ]);

      if (winner.kind === "response") {
        el.cancel();
        const resp = winner.resp;
        if (resp.error) {
          return {
            content: [{ type: "text", text: `Seashail JSON-RPC error: ${resp.error.message}` }],
            details: { ok: false, error: resp.error },
          };
        }
        const rec = asRecord(resp.result) ?? {};
        const contentArr = Array.isArray(rec["content"]) ? (rec["content"] as unknown[]) : [];
        const text = contentArr
          .map((c) => asRecord(c))
          .map((c) => (c && c["type"] === "text" ? String(c["text"] ?? "") : ""))
          .filter(Boolean)
          .join("\n");
        const isError = rec["isError"] === true;
        const parsed = safeJsonParse(text);
        return {
          content: [{ type: "text", text }],
          details: { ok: !isError, payload: parsed ?? text },
        };
      }

      // Elicitation prompt
      const { id: elicitationId, params: p } = winner.req;
      const message = typeof p["message"] === "string" ? (p["message"] as string) : "Seashail requires input.";
      const requestedSchema = p["requestedSchema"] ?? {};

      // Auto-answer passphrase prompts if configured via env var (recommended).
      if (isPassphraseSchema(requestedSchema)) {
        const pw = env[passphraseEnvVar];
        if (typeof pw === "string" && pw.trim()) {
          await client.respondElicitation(elicitationId, { action: "accept", content: { passphrase: pw } });
          continue;
        }
      }

      const token = randToken();
      resumeTokens.set(token, {
        elicitationId,
        pendingToolCall: call.pending,
        toolName,
        toolCallId: call.toolCallId,
        message,
        requestedSchema,
      });

      const guidance =
        isPassphraseSchema(requestedSchema)
          ? `Seashail needs a passphrase. Prefer setting ${passphraseEnvVar} in the OpenClaw gateway env (or ~/.openclaw/.env) instead of pasting secrets into chat.`
          : `Seashail needs confirmation/input. Provide the requested fields via seashail_resume.`;

      return {
        content: [
          {
            type: "text",
            text:
              JSON.stringify(
                {
                  ok: true,
                  status: "needs_approval",
                  tool: toolName,
                  token,
                  message,
                  requestedSchema,
                  guidance,
                },
                null,
                2,
              ) + `\n\nNext: call seashail_resume with token="${token}", action="accept" (or "decline"), and content matching requestedSchema.`,
          },
        ],
        details: {
          ok: true,
          status: "needs_approval",
          tool: toolName,
          token,
          message,
          requestedSchema,
        },
      };
    }
  }

  const toolSpecs = loadToolSpecsFromRepoSchema();
  const registeredToolNames: string[] = [
    toolPrefix ? `${prefix}resume` : "seashail_resume",
    ...toolSpecs
      .map((s) => s.name.trim())
      .filter(Boolean)
      .map((n) => (toolPrefix ? `${prefix}${n}` : n)),
  ];

  // Register tools synchronously at plugin-load time. OpenClaw's `/tools/invoke`
  // path only sees tools declared via `api.registerTool(...)` factories; tools
  // registered inside `registerService.start()` won't be available there.
  api.registerTool(
    (ctx: any) => {
      if (ctx?.sandboxed) return null;

      const tools: any[] = [];

      // Control tool to resume pending Seashail elicitations.
      tools.push({
        name: toolPrefix ? `${prefix}resume` : "seashail_resume",
        label: toolPrefix ? `${prefix}resume` : "seashail_resume",
        description:
          "Resume a Seashail tool call that is waiting for confirmation/input (token provided in the previous tool result).",
        parameters: {
          type: "object",
          additionalProperties: false,
          properties: {
            token: { type: "string" },
            action: { type: "string", enum: ["accept", "decline"] },
            content: {
              type: "object",
              additionalProperties: true,
              description: "Fields requested by Seashail in requestedSchema.",
            },
          },
          required: ["token", "action"],
        },
        execute: async (_id: string, params: Record<string, unknown>) => {
          return await enqueue(async () => {
            const token = typeof params.token === "string" ? params.token : "";
            const action = typeof params.action === "string" ? params.action : "";
            const content = asRecord(params.content) ?? {};

            const st = resumeTokens.get(token);
            if (!st) {
              return {
                content: [{ type: "text", text: `Unknown/expired token: ${token}` }],
                details: { ok: false, error: { type: "unknown_token", message: "Unknown/expired token" } },
              };
            }
            resumeTokens.delete(token);

            await ensureClient();
            await client.respondElicitation(st.elicitationId, { action, content });

            // Continue the original call until it yields again or completes.
            return await runToolUntilYield(st.toolName, {}, { toolCallId: st.toolCallId, pending: st.pendingToolCall });
          });
        },
      });

      // Seashail tools (best-effort list from schema.rs when running from the repo).
      for (const spec of toolSpecs) {
        const toolName = spec.name.trim();
        if (!toolName) continue;
        const name = toolPrefix ? `${prefix}${toolName}` : toolName;
        tools.push({
          name,
          label: name,
          description: spec.description ?? `Seashail tool: ${toolName}`,
          parameters: spec.inputSchema ?? { type: "object", additionalProperties: true },
          execute: async (_id: string, params: unknown) => {
            return await enqueue(async () => await runToolUntilYield(toolName, params));
          },
        });
      }

      return tools;
    },
    // IMPORTANT:
    // When returning an array of tools from a single factory, OpenClaw requires
    // an explicit tool name list for indexing/visibility.
    { optional: true, names: registeredToolNames },
  );

  api.registerService({
    id: "seashail",
    async start() {
      // Eagerly start Seashail MCP so a default wallet exists before any chat/tool narration.
      // This is safe/idempotent: Seashail creates the generated `default` wallet on MCP initialize
      // if no wallets exist yet.
      await enqueue(async () => {
        try {
          await ensureClient();
        } catch {
          // Best-effort: don't prevent the OpenClaw gateway from starting.
          // Tool calls will surface a detailed error if the subprocess cannot start.
        }
      });
    },
    async stop() {
      client.stop();
    },
  });
}

import assert from "node:assert/strict";
import { spawn } from "node:child_process";

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: unknown;
}

function writeJsonLine(p: ReturnType<typeof spawn>, msg: unknown) {
  assert.ok(p.stdin, "expected piped stdin");
  p.stdin.write(`${JSON.stringify(msg)}\n`);
}

function mustRecord(v: unknown, msg: string): Record<string, unknown> {
  assert.ok(v !== null && typeof v === "object" && !Array.isArray(v), msg);
  return v as Record<string, unknown>;
}

function mustArray<T>(v: unknown, msg: string): T[] {
  assert.ok(Array.isArray(v), msg);
  return v as T[];
}

async function rpcCall(
  p: ReturnType<typeof spawn>,
  id: number,
  method: string,
  params: unknown
): Promise<JsonRpcResponse> {
  return await new Promise((resolve, reject) => {
    let stdoutBuf = "";
    const onData = (chunk: Buffer | string) => {
      stdoutBuf += String(chunk);
      while (true) {
        const i = stdoutBuf.indexOf("\n");
        if (i === -1) {
          return;
        }
        const line = stdoutBuf.slice(0, i);
        stdoutBuf = stdoutBuf.slice(i + 1);
        const trimmed = line.trim();
        if (!trimmed) {
          continue;
        }
        let parsed: unknown;
        try {
          parsed = JSON.parse(trimmed) as unknown;
        } catch {
          continue;
        }
        const rec = mustRecord(parsed, "expected jsonrpc object");
        const {
          id: respId,
          jsonrpc,
          result,
          error,
        } = rec as {
          id?: unknown;
          jsonrpc?: unknown;
          result?: unknown;
          error?: unknown;
        };
        if (respId === id) {
          if (jsonrpc !== "2.0" || typeof respId !== "number") {
            continue;
          }
          cleanup();
          resolve({
            jsonrpc: "2.0",
            id: respId,
            result,
            error,
          });
          return;
        }
      }
    };

    const onExit = (code: number | null, signal: NodeJS.Signals | null) => {
      cleanup();
      reject(
        new Error(
          `mcp process exited before response (code=${code}, signal=${signal})`
        )
      );
    };

    const cleanup = () => {
      p.stdout?.off("data", onData);
      p.off("exit", onExit);
    };

    p.stdout?.on("data", onData);
    p.on("exit", onExit);
    writeJsonLine(p, { jsonrpc: "2.0", id, method, params });
  });
}

export async function listSeashailMcpToolNames(opts: {
  seashailBinPath: string;
  env: Record<string, string>;
}): Promise<string[]> {
  const p = spawn(opts.seashailBinPath, ["mcp", "--standalone"], {
    stdio: ["pipe", "pipe", "pipe"],
    env: opts.env,
  });
  p.stdout?.setEncoding("utf8");

  try {
    const init = await rpcCall(p, 1, "initialize", {});
    assert.equal(init.error, undefined, "initialize must succeed");

    const listed = await rpcCall(p, 2, "tools/list", {});
    assert.equal(listed.error, undefined, "tools/list must succeed");
    const result = mustRecord(
      listed.result,
      "tools/list result must be an object"
    );
    const tools = mustArray<Record<string, unknown>>(
      result["tools"],
      "missing tools array"
    );
    return tools
      .map((t) => t["name"])
      .filter((n): n is string => typeof n === "string");
  } finally {
    try {
      p.stdin?.end();
    } catch {
      // ignore
    }
    try {
      p.kill("SIGTERM");
    } catch {
      // ignore
    }
  }
}

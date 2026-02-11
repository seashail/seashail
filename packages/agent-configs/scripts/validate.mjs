import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

/**
 * @param {string} dir
 * @returns {string[]}
 */
function listJsonFiles(dir) {
  /** @type {string[]} */
  const out = [];
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    const st = statSync(p);
    if (st.isDirectory()) {
      for (const child of listJsonFiles(p)) out.push(child);
      continue;
    }
    if (entry.endsWith(".json")) out.push(p);
  }
  return out;
}

/**
 * @param {unknown} v
 * @param {string} msg
 * @returns {Record<string, unknown>}
 */
function mustObject(v, msg) {
  if (!v || typeof v !== "object" || Array.isArray(v)) throw new Error(msg);
  return /** @type {Record<string, unknown>} */ (v);
}

/**
 * @param {unknown} v
 * @param {string} msg
 * @returns {string[]}
 */
function mustStringArray(v, msg) {
  if (!Array.isArray(v) || v.some((x) => typeof x !== "string")) throw new Error(msg);
  return /** @type {string[]} */ (v);
}

const root = join(import.meta.dirname, "..", "templates");
const files = listJsonFiles(root);
if (files.length === 0) throw new Error("no templates found");

for (const file of files) {
  const raw = JSON.parse(readFileSync(file, "utf8"));
  const obj = mustObject(raw, `template must be an object: ${file}`);

  const servers =
    obj["mcpServers"] ??
    obj["servers"] ??
    (() => {
      throw new Error(`template missing servers key (mcpServers/servers): ${file}`);
    })();

  const serversObj = mustObject(servers, `servers must be an object: ${file}`);
  const seashail = serversObj["seashail"];
  const seashailObj = mustObject(seashail, `missing servers.seashail object: ${file}`);

  const command = seashailObj["command"];
  if (command !== "seashail") throw new Error(`servers.seashail.command must be "seashail": ${file}`);

  const args = mustStringArray(seashailObj["args"], `servers.seashail.args must be string[]: ${file}`);
  if (!args.includes("mcp")) throw new Error(`servers.seashail.args must include "mcp": ${file}`);
}

process.stdout.write(`ok: validated ${files.length} templates\n`);

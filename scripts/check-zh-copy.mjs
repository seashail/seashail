#!/usr/bin/env node

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOTS = [
  "README.zh-CN.md",
  "apps/docs/content/docs",
  "apps/docs/content/glossary/zh-CN.json",
  "apps/landing/src/content/copy.zh.ts",
];

const FORBIDDEN = [
  { label: "自动审批", regex: /自动审批/g },
  { label: "硬性拒绝", regex: /硬性拒绝/g },
  { label: "硬性拦截", regex: /硬性拦截/g },
  { label: "健全性检查", regex: /健全性检查/g },
  { label: "包装器", regex: /包装器/g },
  { label: "脑裂状态", regex: /脑裂状态/g },
  { label: "原生路径", regex: /原生路径/g },
  { label: "交易封装", regex: /交易封装/g },
  { label: "工具封装", regex: /工具封装/g },
  { label: "用于策略执行的尽力美元价值", regex: /用于策略执行的尽力美元价值/g },
  { label: "提款", regex: /提款/g },
];

const REQUIRED = [
  "智能体",
  "自动批准",
  "硬性阻止",
  "原生执行路径",
  "交易载荷",
  "工具调用载荷",
  "策略评估用的估算美元价值",
];

function listFiles(path) {
  const st = statSync(path);
  if (st.isFile()) return [path];
  const out = [];
  for (const entry of readdirSync(path)) {
    out.push(...listFiles(join(path, entry)));
  }
  return out;
}

function isTargetFile(path) {
  if (path === "README.zh-CN.md") return true;
  if (path.endsWith(".zh.mdx")) return true;
  if (path.endsWith("meta.zh.json")) return true;
  if (path.endsWith("copy.zh.ts")) return true;
  if (path.endsWith("zh-CN.json")) return true;
  return false;
}

function stripMarkdown(line, inFence) {
  if (/^\s*```/.test(line) || /^\s*~~~/.test(line)) {
    return { text: "", nextFence: !inFence };
  }
  if (inFence) return { text: "", nextFence: inFence };
  const text = line.replace(/`[^`]*`/g, "");
  return { text, nextFence: inFence };
}

const files = ROOTS.flatMap((p) => listFiles(p)).filter(isTargetFile);
const failures = [];
const requiredCounts = new Map(REQUIRED.map((t) => [t, 0]));

for (const file of files) {
  const raw = readFileSync(file, "utf8");
  const lines = raw.split("\n");
  const isMarkdown = file.endsWith(".mdx") || file.endsWith(".md");
  let inFence = false;

  for (let i = 0; i < lines.length; i++) {
    const lineNo = i + 1;
    const line = lines[i];
    const processed = isMarkdown ? stripMarkdown(line, inFence) : { text: line, nextFence: inFence };
    inFence = processed.nextFence;
    const scan = processed.text;
    if (!scan) continue;

    for (const term of REQUIRED) {
      const n = (scan.match(new RegExp(term, "g")) || []).length;
      if (n > 0) requiredCounts.set(term, requiredCounts.get(term) + n);
    }

    for (const rule of FORBIDDEN) {
      if (rule.regex.test(scan)) {
        failures.push(`${file}:${lineNo} contains forbidden term: ${rule.label}`);
      }
      rule.regex.lastIndex = 0;
    }
  }
}

for (const [term, count] of requiredCounts.entries()) {
  if (count === 0) failures.push(`missing required term in corpus: ${term}`);
}

if (failures.length > 0) {
  console.error("zh-copy check failed:");
  for (const f of failures) console.error(`- ${f}`);
  process.exit(1);
}

console.log(`zh-copy check passed (${files.length} files scanned).`);

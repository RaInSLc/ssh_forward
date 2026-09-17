#!/usr/bin/env node
// 校验版本号在四处定义中保持一致，并确认 CHANGELOG 有对应条目。
//
// 背景：`AGENTS.md`「版本发布与版本号递增规则」要求版本号必须同步更新以下全部文件：
//   1. 根目录 Cargo.toml          ([workspace.package] version)
//   2. apps/desktop/src-tauri/tauri.conf.json  ("version")
//   3. apps/desktop/package.json               ("version")
//   4. CHANGELOG.md               (对应的版本号标题)
// 该规则此前靠人工执行，容易漏改。本脚本把它变成可自动化的检查。
//
// 用法：
//   node scripts/check-version.mjs
// 退出码：0 = 全部一致；1 = 存在不一致或缺失。
//
// 无第三方依赖，仅用 Node 内置模块。

import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** 从根 Cargo.toml 的 [workspace.package] 段读取 version。 */
function readCargoVersion() {
  const path = join(repoRoot, "Cargo.toml");
  const text = readFileSync(path, "utf8");
  const lines = text.split(/\r?\n/);

  let inSection = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith("[")) {
      inSection = trimmed === "[workspace.package]";
      continue;
    }
    if (!inSection) continue;
    const match = /^version\s*=\s*"([^"]+)"/.exec(trimmed);
    if (match) return { value: match[1], source: "Cargo.toml ([workspace.package])" };
  }
  throw new Error("未能在 Cargo.toml 的 [workspace.package] 段找到 version");
}

/** 读取一个 JSON 文件的顶层 version 字段。 */
function readJsonVersion(relativePath) {
  const path = join(repoRoot, relativePath);
  const data = JSON.parse(readFileSync(path, "utf8"));
  if (typeof data.version !== "string") {
    throw new Error(`${relativePath} 缺少字符串类型的 version 字段`);
  }
  return { value: data.version, source: relativePath };
}

/** 收集 CHANGELOG.md 中所有 "## [x.y.z]" 形式的版本标题。 */
function readChangelogVersions() {
  const path = join(repoRoot, "CHANGELOG.md");
  const text = readFileSync(path, "utf8");
  const versions = [];
  for (const line of text.split(/\r?\n/)) {
    const match = /^##\s+\[([^\]]+)\]/.exec(line.trim());
    if (match) versions.push(match[1]);
  }
  return versions;
}

function main() {
  const sources = [
    readCargoVersion(),
    readJsonVersion("apps/desktop/src-tauri/tauri.conf.json"),
    readJsonVersion("apps/desktop/package.json"),
  ];

  const changelogVersions = readChangelogVersions();

  const unique = [...new Set(sources.map((s) => s.value))];

  console.log("版本号定义：");
  for (const { value, source } of sources) {
    console.log(`  ${value.padEnd(12)} ${source}`);
  }
  console.log(`\nCHANGELOG.md 中的版本标题（最近 5 个）：`);
  for (const version of changelogVersions.slice(0, 5)) {
    console.log(`  ${version}`);
  }

  const problems = [];

  // 1) 三个字面量必须一致。
  if (unique.length !== 1) {
    problems.push(
      `三处版本号不一致：${sources.map((s) => `${s.source}=${s.value}`).join("，")}`
    );
  }

  const version = unique[0] ?? sources[0].value;

  // 2) 版本号必须符合 semver 的 x.y.z 形式。
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    problems.push(`版本号 "${version}" 不是 x.y.z 形式的 semver`);
  }

  // 3) CHANGELOG 必须有当前版本的条目。
  if (!changelogVersions.includes(version)) {
    problems.push(`CHANGELOG.md 缺少当前版本 [${version}] 的条目`);
  }

  // 4) 同一版本不得在 CHANGELOG 中出现多次（AGENTS.md 禁止重复发布同一版本）。
  const duplicates = changelogVersions.filter(
    (v, index) => changelogVersions.indexOf(v) !== index
  );
  if (duplicates.length > 0) {
    problems.push(`CHANGELOG.md 存在重复的版本标题：[${[...new Set(duplicates)].join(", ")}]`);
  }

  console.log("");
  if (problems.length > 0) {
    for (const problem of problems) console.error(`✗ ${problem}`);
    console.error(`\n版本号校验失败（共 ${problems.length} 项）。`);
    process.exit(1);
  }

  console.log(`✓ 版本号校验通过：四处定义一致，当前版本 ${version}。`);
}

main();

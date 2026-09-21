#!/usr/bin/env node
// 校验前端与 Rust 之间的 IPC 契约：命令名必须双向一致。
//
// 背景（二次设计评审 A2）：前端手写 TS 类型、以字符串命令名调用 Tauri，
// 与 Rust 侧 `generate_handler!` 之间没有任何自动约束。重命名一个命令，
// Rust 编译通过、`tsc --noEmit` 也通过，只在运行时以「命令不存在」暴露。
// 现状已产生可见症状：`set_config_path` 与 `validate_config` 注册后前端零引用。
//
// 本脚本做两组检查：
//   1) 死命令  ：`generate_handler!` 注册但前端从不引用（须在 KNOWN_UNUSED 中登记并写明原因）
//   2) 未知命令：前端引用但 `generate_handler!` 未注册（拼写错误 / 已删除）
//
// 提取规则（关键，不能简化）：前端命令名并不都以字面量直接传给 `invoke`，
// 而是经由统一包装器 `action(command, payload)`，且部分调用点用三元表达式：
//
//     void action(running ? "stop_tunnel" : "start_tunnel", { name });
//
// 因此规则是「取 `action(` / `invoke(` / `invoke<…>(` 的**第一个顶层逗号之前**的全部
// 字符串字面量」。这样包装器自身的 `invoke(command, payload)` 不会贡献任何字面量
// （实参是标识符而非字面量），而三元表达式两侧的命令名都会被捕获。
//
// 已知局限：第一个实参内若出现含 `(` `)` `,` 的字符串，分段会提前结束。
// 当前所有调用点均无此情形；将来出现时脚本会**少报**而非误报，需一并调整分段逻辑。
//
// 用法：node scripts/check-ipc-contract.mjs
// 退出码：0 = 一致；1 = 存在不一致。无第三方依赖，仅用 Node 内置模块。

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const MAIN_RS = "apps/desktop/src-tauri/src/main.rs";
const FRONTEND_SRC = "apps/desktop/src";

/**
 * 允许「已注册但前端未引用」的命令，**必须写明原因**。
 *
 * 这不是豁免区：一旦某条命令重新被前端引用，脚本会报「过期登记」并失败，
 * 迫使本表保持精确。目标是让每一条不可达命令都是**已知且有意**的，
 * 而不是悄悄累积。每条命令的最终去向（接线 or 删除）属产品决策。
 */
const KNOWN_UNUSED = new Map([
  [
    "set_config_path",
    "G1 未实现：界面没有「选择配置文件」入口，暂保留命令以备接线",
  ],
  [
    "validate_config",
    "界面无「校验配置」入口；各写入路径已各自调用 validate()，暂保留命令以备接线",
  ],
]);

/** 命令名的形状：snake_case。用于把「看起来像命令」的字面量与普通字符串区分开。 */
const COMMAND_LIKE = /^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$/;

/** 前端调用点：`action(` / `invoke(` / `invoke<T>(`，排除 `xxx.action(` 这类成员调用。 */
const CALL_SITE = /(?<![\w.$])(action|invoke)\s*(?:<[^<>()]*>)?\s*\(/g;

/** 双引号字符串字面量（不跨行）。 */
const STRING_LITERAL = /"((?:[^"\\\n]|\\.)*)"/g;

/** 从 `main.rs` 的 `generate_handler![...]` 中读出注册的命令名。 */
function readRegisteredCommands() {
  const source = readFileSync(join(repoRoot, MAIN_RS), "utf8");
  const match = /generate_handler!\s*\[([\s\S]*?)\]/.exec(source);
  if (!match) {
    throw new Error(`未能在 ${MAIN_RS} 中找到 generate_handler![...]`);
  }
  return match[1]
    .split(",")
    .map((entry) => entry.replace(/\/\/[^\n]*/g, "").trim())
    .filter(Boolean);
}

/** 递归收集前端源码文件（.ts / .tsx）。 */
function collectFrontendFiles(dir) {
  const found = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      found.push(...collectFrontendFiles(full));
    } else if (/\.tsx?$/.test(entry.name)) {
      found.push(full);
    }
  }
  return found;
}

/**
 * 取第一个实参的源码片段。
 *
 * 从开括号处开始按括号深度扫描：遇到顶层逗号（深度回到 1）或与之配对的闭括号即停。
 */
function firstArgumentSegment(source, openParenIndex) {
  let depth = 0;
  for (let index = openParenIndex; index < source.length; index += 1) {
    const char = source[index];
    if (char === "(") {
      depth += 1;
    } else if (char === ")") {
      depth -= 1;
      if (depth === 0) return source.slice(openParenIndex + 1, index);
    } else if (char === "," && depth === 1) {
      return source.slice(openParenIndex + 1, index);
    }
  }
  return source.slice(openParenIndex + 1);
}

/**
 * 扫描前端源码，收集「看起来像命令名」的实参字面量。
 *
 * 返回 `Map<命令名, Set<相对路径>>`，路径用于报错时定位。
 */
function readReferencedCommands() {
  const references = new Map();
  const files = collectFrontendFiles(join(repoRoot, FRONTEND_SRC));

  for (const file of files) {
    const source = readFileSync(file, "utf8");
    const shortPath = relative(repoRoot, file).replace(/\\/g, "/");

    for (const call of source.matchAll(CALL_SITE)) {
      const openParenIndex = call.index + call[0].length - 1;
      const segment = firstArgumentSegment(source, openParenIndex);

      for (const literal of segment.matchAll(STRING_LITERAL)) {
        const value = literal[1];
        if (!COMMAND_LIKE.test(value)) continue;
        if (!references.has(value)) references.set(value, new Set());
        references.get(value).add(shortPath);
      }
    }
  }

  return references;
}

function main() {
  const registered = readRegisteredCommands();
  const referenced = readReferencedCommands();

  const duplicates = registered.filter(
    (name, index) => registered.indexOf(name) !== index,
  );
  const registeredSet = new Set(registered);

  // 死命令：注册但零引用。已登记的豁免不算问题；已登记却重新被引用的算「过期登记」。
  const unused = [];
  const stale = [];
  for (const name of registered) {
    const isReferenced = referenced.has(name);
    const isExempt = KNOWN_UNUSED.has(name);
    if (!isReferenced && !isExempt) unused.push(name);
    if (isReferenced && isExempt) stale.push(name);
  }

  // 未知命令：前端引用但未注册。
  const unknown = [...referenced.keys()]
    .filter((name) => !registeredSet.has(name))
    .sort();

  console.log(`Rust 注册命令（${registered.length} 个，来自 ${MAIN_RS}）：`);
  console.log(`  ${registered.join(", ")}\n`);

  const exempted = registered.filter((name) => KNOWN_UNUSED.has(name));
  if (exempted.length > 0) {
    console.log("已登记为「暂不可达」的命令：");
    for (const name of exempted) {
      console.log(`  ${name} — ${KNOWN_UNUSED.get(name)}`);
    }
    console.log("");
  }

  console.log(`前端引用的命令（${referenced.size} 个）：`);
  for (const [name, files] of [...referenced].sort()) {
    console.log(`  ${name.padEnd(26)} ${[...files].join(", ")}`);
  }

  const problems = [];
  if (duplicates.length > 0) {
    problems.push(
      `generate_handler! 中重复注册：[${[...new Set(duplicates)].join(", ")}]`,
    );
  }
  if (unused.length > 0) {
    problems.push(
      `注册但前端零引用的死命令：[${unused.join(", ")}]` +
        "（若确为有意保留，请加入脚本内的 KNOWN_UNUSED 并写明原因）",
    );
  }
  if (unknown.length > 0) {
    problems.push(
      `前端引用但未注册的命令：[${unknown.join(", ")}]（拼写错误或已被删除）`,
    );
  }
  if (stale.length > 0) {
    problems.push(
      `KNOWN_UNUSED 中的过期登记：[${stale.join(", ")}]（已被前端引用，请从表中移除）`,
    );
  }

  console.log("");
  if (problems.length > 0) {
    for (const problem of problems) console.error(`✗ ${problem}`);
    console.error(`\nIPC 契约校验失败（共 ${problems.length} 项）。`);
    process.exit(1);
  }

  console.log(
    `✓ IPC 契约校验通过：${registered.length} 个命令双向一致` +
      `（其中 ${exempted.length} 个为已登记的暂不可达命令）。`,
  );
}

main();

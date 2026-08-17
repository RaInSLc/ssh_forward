---
name: cicd-troubleshooting
description: >-
  用于排查、诊断并修复 GitHub Actions CI/CD 发布与构建流水线失败的专用技能。
  当用户遇到 CI/CD 报错、Release 打包失败、跨平台编译异常或在提交发布前需要进行发布前置合规自检时使用此 Skill。
---

# CI/CD 故障排查与发布流水线自检指南

本技能总结了本项目（SSH Forward 及基于 Tauri + Rust + React 的跨平台项目）在 GitHub Actions 发布流水线中经常出现的失败根因、诊断方法及修复规程。

---

## 一、 CI/CD 经常失败的核心问题点

根据历次 GitHub Actions 构建日志（[Workflow 历史](https://github.com/RaInSLc/ssh_forward/actions)），常见失败可划分为以下 5 大类：

### 1. 代码格式不符合 `cargo fmt`（最频发，约 40% 失败率）
- **现象**：CI 在运行 `cargo fmt --all -- --check` 阶段立即报错退出（Exit code 1）。
- **根因**：
  - 新建或修改 Rust 文件后，`use` 模块导入语句未按字母排序或格式存在冗余空行。
  - Windows 本地编辑产生的格式与 CI 环境下的 rustfmt 规则微小冲突。
- **排查与修复**：
  - 提交前必须在本地根目录运行：`cargo fmt --all`。
  - 验证命令：`cargo fmt --all -- --check`，确保零输出。

---

### 2. Clippy 静态检查报警（`-D warnings` 强制拦截）
- **现象**：CI 在 `cargo clippy --workspace -- -D warnings` 阶段阻断构建。
- **典型告警类型**：
  - **`clippy::collapsible_if`**：多层嵌套 `if` 可以合并（如 `if let Ok(x) = ... { if x.success() ... }` 需简化为 `.is_ok_and(...)` 或单层判断）。
  - **`clippy::unused_variables` / `clippy::unused_imports`**：引入了多余未使用的变量或依赖。
  - **`clippy::needless_borrow` / `clippy::redundant_closure`**：冗余引用或闭包传递。
- **排查与修复**：
  - 提交前本地必须运行：`cargo clippy --workspace -- -D warnings`。

---

### 3. 跨平台编译兼容性错误（Windows vs macOS Intel / Apple Silicon）
- **现象**：Windows 本地编译通过，但 macOS (macOS-15-intel / macOS-14 aarch64) Runner 编译失败。
- **典型原因**：
  - Windows 特有 API（如 `Win32 CryptProtectData`、`CommandExt::creation_flags` 等）未加 `#[cfg(windows)]` 条件编译隔离。
  - 缺乏非 Windows 平台的降级实现（例如提供 `#[cfg(not(windows))]` 的同名函数或 Stub 实现）。
  - 路径硬编码反斜杠 `\` 在 Unix/macOS 环境下无法解析。
- **排查与修复**：
  - 涉及 OS 底层特性的代码必须使用条件编译：
    ```rust
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    ```
  - 跨平台路径操作必须使用 `std::path::Path` 或 `PathBuf::join()`。

---

### 4. 前端 TypeScript 类型检查与构建失败
- **现象**：`npm run check` (`tsc --noEmit`) 或 `npm run build` 失败。
- **典型原因**：
  - React/TypeScript 代码存在未定义的类型、未使用的 prop 或类型不匹配。
  - `package-lock.json` 与 `package.json` 版本不一致导致 `npm ci` 阶段失败。
- **排查与修复**：
  - 在 `apps/desktop` 下运行：`npm run check ; npm run build`。

---

### 5. 版本号与 Git Tag 冲突或资产未生成
- **现象**：Tauri 构建完成但发布资产上传缺失，或者 Tag 覆盖导致 Actions 行为异常。
- **关键规则**：
  - 项目四大版本号必须严格同步：
    1. 根目录 `Cargo.toml` (`[workspace.package] version`)
    2. `apps/desktop/package.json` (`version`)
    3. `apps/desktop/src-tauri/tauri.conf.json` (`version`)
    4. `apps/desktop/src/App.tsx` (`version` 常量)
  - 每次发布新版本必须在 `CHANGELOG.md` 中记录变更日志。

---

## 二、 自动化发布前“四步合规自检”标准流程 (Pre-Push Checklist)

在推送 Git Tag 触发 CI/CD 之前，**必须按顺序在本地执行以下自检**：

```powershell
# 1. 自动格式化并校验 Rust 代码
cargo fmt --all ; cargo fmt --all -- --check

# 2. 运行 Clippy 零警告检查
cargo clippy --workspace -- -D warnings

# 3. 运行全量单元测试
cargo test -p ssh-forward-config -p ssh-forward-ssh -p ssh-forward-core

# 4. 前端类型检查与打包验证
npm --prefix apps\desktop run check ; npm --prefix apps\desktop run build
```

只有当上述 4 步均返回 `exit code 0` 时，才允许执行打 Tag 与推送操作：

```powershell
git add .
git commit -m "feat/fix: <提交说明>"
git tag vX.Y.Z
git push origin main --tags
```

---

## 三、 CI/CD 故障排查标准操作步骤 (Runbook)

当收到用户报告 CI/CD 失败时，执行以下排查规程：

1. **获取失败阶段**：
   - 访问 GitHub Actions 运行页面或 API（`https://api.github.com/repos/RaInSLc/ssh_forward/actions/runs`）。
   - 查看是在 `build (windows-latest)`、`build (macos-15-intel)` 还是 `build (macos-14)` 阶段失败。
2. **定位失败步骤**：
   - 若在 30 秒~1 分钟内所有 Runner 均失败，通常是 `cargo fmt` 或 `cargo clippy` 拦截。
   - 若仅 macOS Runner 失败，通常是 `#[cfg(windows)]` 平台专属 API 泄漏或跨平台编译缺少 Stub。
   - 若在 `Build desktop bundles` 失败，检查 Tauri 配置文件 `tauri.conf.json` 或前端资源缺失。
3. **修复与重新发布**：
   - 针对根因修改代码，执行本地自检。
   - 删除旧 Tag 并推送更新后的 Tag：
     ```powershell
     git tag -d vX.Y.Z ; git push origin :refs/tags/vX.Y.Z ; git tag vX.Y.Z ; git push origin main --tags
     ```

# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- 集成 Tauri v2 官方在线更新机制（`tauri-plugin-updater` & `@tauri-apps/plugin-updater`）。
- 桌面端关于弹窗支持检查 GitHub Releases 更新、展示版本更新说明、下载进度条与一键更新重启。
- GitHub Actions CI/CD 流水线支持私钥数字签名打包并自动生成跨平台 `latest.json` 更新清单资产。
- 转发配置本地地址支持下拉选择（`127.0.0.1`、`0.0.0.0`、`localhost`）。
- 新建 Tunnel 支持自动探测并分配空闲随机端口，表单新增“🎲 随机”端口分配按钮。
- 后端新增 `get_available_port` 系统空闲端口原子探测命令。
- `.gitignore` 增加 `.tauri/` 和私钥文件隔离规则。

### Fixed
- 修复 Windows 下软件意外关闭或直接关闭导致子进程 `ssh.exe` 变成孤儿进程持续占用端口的问题（引入 Windows 内核 Job Object 机制 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + Tauri 窗口与退出生命周期双重自动清理）。
- 修复关于弹窗中版本号未动态获取的问题，由后端 `Snapshot` 动态返回 `env!("CARGO_PKG_VERSION")`。
- 优化检查更新异常捕获，屏蔽云端暂未上传本平台清单时的报错提示，提供友好的“已是最新版本”提示。

## [0.1.11] - 2026-08-17

### Fixed
- 修复 Windows 下启动 Tunnel 时弹出 `ssh-keyscan.exe` 黑色控制台窗口的问题（增加 `CREATE_NO_WINDOW` 标记）。
- 修复 `ssh-keyscan` 强制限定 `-t ed25519` 导致 RSA/ECDSA 密钥类型服务器无法获取 Host Key 并启动失败的问题。
- 将 OpenSSH 连接策略更新为 `StrictHostKeyChecking=accept-new`，实现首次连接自动安全登记 Host Key，避免无谓阻塞与扫描失败拦截。
- 完善 `start_tunnel` 启动前置阶段的错误捕获与状态同步机制。

## [0.1.10] - 2026-08-15

### Added
- 桌面端 UI 增加删除服务器 (Host) 和删除转发 (Tunnel) 的操作按钮与二次确认防呆逻辑。
- 服务器删除前增加关联 Tunnel 关联度的校验拦截提示。
- README.md 新增 CLI 命令行工具使用说明指南、Windows 安装版配置文件保存路径说明以及 DPAPI 密码跨设备解密限制提示。

### Changed
- 升级项目 workspace 及桌面端应用版本号至 0.1.10。

## [0.1.9] - 2026-08-13

### Added
- 初始版本发布，支持 SSH Local Forward 端口转发管理与桌面控制台。

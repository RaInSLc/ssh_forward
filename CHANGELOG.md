# Changelog

All notable changes to this project will be documented in this file.

## [0.1.16] - 2026-08-18

### Optimized
- **极窄窗口自适应与表格列重构**：
  - 将表格拆分的「本地端口」与「远端目标」合并为单列「转发链路 / 路由」，将表格精简为 4 列，彻底消除横向滚动条。
  - 侧边栏宽度与工作区内边距弹性紧凑化，在 800px 甚至更窄窗口下 100% 完整显示所有信息与操作按钮。
  - 窗口最小安全尺寸调整为 `800x560`，默认尺寸 `1060x720`。

## [0.1.15] - 2026-08-18

### Added
- **自适应响应式布局与表格视图**：针对窄屏或小窗口宽度自动优化降级为紧凑表格列表视图（Table View），彻底修复按钮文字换行折叠和排版拥挤问题，并支持卡片/表格视图手动切换。
- **基础模式 vs 高级设置模式分级**：
  - **基础模式 (默认)**：保持 `0.1.13` 的极简体验，界面无干扰、表单简洁专注。
  - **高级设置模式 (一键切换)**：开启后全面解锁 `0.1.14` 的 SOCKS5 动态代理、远程反向穿透、跳板机、证书、数据压缩与全局保活等专业网络设置。

## [0.1.14] - 2026-08-18

### Added
- **连接保活与高可用防假死**：引入 `ServerAliveInterval`（默认 15s）、`ServerAliveCountMax`（默认 3 次）与 `TCPKeepAlive`（默认开启），彻底解决 NAT/防火墙静默丢包导致的 SSH 假死。
- **动态端口转发 (SOCKS5 代理 `-D`)**：新增 `Dynamic` 隧道模式，仅需在本地指定端口即可建立全功能 SOCKS5 代理网关，直接访问远程全网服务。
- **远程反向端口转发 (`-R`)**：新增 `Remote` 隧道模式，支持内网穿透将本地服务端口暴露到公网服务器。
- **跳板机与代理穿透**：主机支持关联级联跳板机（`ProxyJump` / `-J`）与自定义前置代理命令（`ProxyCommand`）。
- **局域网共享支持**：支持 `GatewayPorts` / 绑定 `0.0.0.0`，允许局域网同伴设备直接访问转发端口。
- **认证隔离与安全进阶**：支持 `IdentitiesOnly=yes`（避免遍历 SSH-Agent 触发认证次数超限）及 CA 签发的用户证书认证（`CertificateFile`）。
- **传输性能优化**：支持数据流压缩（`Compression` / `-C`），提升高延迟/弱网传输速率。
- **自定义 OpenSSH 参数透传**：主机和隧道均支持配置任意自定义 `-o Option=Value` 参数列表。
- **界面与交互重塑**：新增模式分段切换、折叠式高级设置 Accordion、全局网络设置抽屉及彩色模式徽章。

## [0.1.13] - 2026-08-17

### Added
- 集成 Tauri v2 官方在线更新机制（`tauri-plugin-updater` & `@tauri-apps/plugin-updater`）。
- 桌面端关于弹窗支持检查 GitHub Releases 更新、展示版本更新说明、下载进度条与一键更新重启。
- GitHub Actions CI/CD 流水线支持私钥数字签名打包并自动生成跨平台 `latest.json` 更新清单资产。
- 转发配置本地地址支持下拉选择（`127.0.0.1`、`0.0.0.0`、`localhost`）。
- 新建 Tunnel 支持自动探测并分配空闲随机端口，表单新增“🎲 随机”端口分配按钮。
- 后端新增 `get_available_port` 系统空闲端口原子探测命令。
- 添加服务器默认认证方式调整为“密码”；增加 SSH Agent 前置使用条件说明与私钥/密码输入提示。
- Tunnel 启动失败时根据认证类型输出精准诊断指引，并在错误卡片提供一键跳转编辑服务器的快捷入口。
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

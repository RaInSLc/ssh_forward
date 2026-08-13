# SSH Forward

`ssh-forward` 是一个使用 Rust 编写的命令行基础工具，用于管理安全的本地 SSH 端口转发。本次初始实现遵循项目设计中“核心优先”的开发顺序：JSON 配置、配置校验、Host 与 Local Tunnel 的增删改查，以及兼容 OpenSSH 的本地转发。

## 当前功能范围

- 使用严格、带版本号的 JSON 配置，并通过原子写入降低配置损坏风险。
- 支持使用 SSH Agent 或私钥路径管理 Host 的增删改查；配置中不会保存密码或私钥内容。
- 支持 Local（`-L`）Tunnel 的增删改查，仅允许绑定回环地址，并会在启动前检查本地端口可用性。
- 通过 OpenSSH 创建转发，默认使用 `ExitOnForwardFailure=yes` 与 `StrictHostKeyChecking=yes`。

桌面端界面、系统托盘、内嵌 SSH 后端、系统凭据库、远程/动态转发、自动重连运行时和守护进程属于后续工作。

## 环境要求

- Rust 工具链（当前已使用 Rust 1.96+ 与 Cargo）。
- Node.js 22+ 与 npm 10+，仅桌面端开发和打包需要。
- Windows 桌面端还需要 WebView2 Runtime 与 Visual Studio C++ Build Tools（包含 MSVC 和 Windows SDK）。
- 可选：Windows OpenSSH。CLI 的实际 Local Tunnel 启动会调用系统 `ssh.exe`。

以下命令均以项目根目录为当前目录：

```powershell
Set-Location E:\IdeaProjects\pythonProject\ssh_forward
```

项目将 Cargo 下载缓存写入根目录 `.cargo`，避免使用当前用户目录作为项目构建缓存：

```powershell
$env:CARGO_HOME = "$PWD\.cargo"
```

## 首次准备

CLI 不需要 npm 依赖。使用桌面端前，安装项目内前端与 Tauri CLI 依赖：

```powershell
Set-Location apps\desktop
npm install
Set-Location ..\..
```

依赖将写入 `apps/desktop/node_modules`，锁定版本记录在 `apps/desktop/package-lock.json`。

## CLI 编译与启动

### Debug 编译

```powershell
$env:CARGO_HOME = "$PWD\.cargo"
cargo build -p ssh-forward
```

生成的可执行文件为 `target\debug\ssh-forward.exe`。直接运行：

```powershell
.\target\debug\ssh-forward.exe --help
```

也可以由 Cargo 编译并启动：

```powershell
$env:CARGO_HOME = "$PWD\.cargo"
cargo run -p ssh-forward -- --help
```

### Release 编译

```powershell
$env:CARGO_HOME = "$PWD\.cargo"
cargo build -p ssh-forward --release
```

Release 可执行文件为 `target\release\ssh-forward.exe`。

### CLI 使用示例

除非通过 `--config` 指定配置文件，否则 CLI 使用当前工作目录的 `config.json`。

```powershell
.\target\debug\ssh-forward.exe host add development --host ssh.example.invalid --user developer
.\target\debug\ssh-forward.exe tunnel add jupyter --host development --local 127.0.0.1:18888 --remote 127.0.0.1:8888
.\target\debug\ssh-forward.exe config validate
.\target\debug\ssh-forward.exe start jupyter
```

使用 `host add` 时可通过 `--key path\to\private_key` 指定私钥路径；省略该参数时使用 SSH Agent。`start` 会在前台运行；按 `Ctrl+C` 可终止 OpenSSH 子进程。仓库不包含实际 SSH 目标主机；`examples/config.json` 使用不可路由的示例地址，可安全查看和校验。

## 配置

JSON Schema 参考文件位于 `schemas/config-v1.schema.json`，安全的代表性配置示例位于 `examples/config.json`。

V0.1 仅接受 `local` 类型的 Tunnel，本地绑定地址必须为 `127.0.0.1` 或 `::1`，以防止意外暴露给局域网中的其他设备。

### 认证与密码安全

添加或编辑服务器时可选择 SSH Agent、私钥路径或密码认证。

- SSH Agent 不保存认证秘密。
- 私钥认证只保存私钥文件路径，不保存私钥内容或口令。
- Windows 上的密码认证使用 DPAPI 加密后保存到 `config.json` 的 `auth.encrypted_password` 字段；文件中不保存明文密码。

DPAPI 密文只能由保存密码的 Windows 用户在原机器上解密。编辑使用密码认证的服务器时，密码输入框留空会保留已有密文；输入新密码会重新加密并替换密文。旧版本只保存 `credential_id` 的密码服务器无法恢复密码，请编辑服务器并重新输入一次密码完成迁移。

### Host Key 确认

首次启动某个 Host 的 Tunnel 时，桌面端会自动执行与 OpenSSH 输入 `yes` 相同的确认流程：获取并保存服务器的 ED25519 Host Key 到当前 Windows 用户的 `~/.ssh/known_hosts`，然后继续严格校验连接。后续连接仍使用严格校验；若服务器 Host Key 发生变化，程序会拒绝连接，不会自动覆盖原记录。

### 浏览器与主题

Tunnel 卡片提供“打开浏览器”按钮，会使用系统默认浏览器访问本地转发的 HTTP 地址。编辑 Tunnel 时可勾选“启动成功后自动打开浏览器”；旧 Tunnel 默认关闭该选项。页头可在日间模式和夜间模式之间切换，选择保存在当前设备的桌面端本地存储中。

## 桌面端 GUI

桌面端位于 `apps/desktop`，基于 Tauri 2、React 和 TypeScript。它与 CLI 复用相同的 Rust 配置和 Core 模块，可查看、校验和管理 Host 及 Local Tunnel 配置。

### 开发启动（热更新）

首次安装 npm 依赖后，在桌面端目录运行项目内 Tauri CLI：

```powershell
Set-Location apps\desktop
.\node_modules\.bin\tauri.cmd dev
```

该命令会自动启动 Vite 开发服务器（`127.0.0.1:1420`）并打开桌面窗口；修改 React、TypeScript 或 CSS 后会热更新。按 `Ctrl+C` 停止开发服务器和桌面进程。

### Release 编译与 Windows 打包

以下命令构建可直接运行、加载本地前端资源的桌面端可执行程序：

```powershell
Set-Location apps\desktop
.\node_modules\.bin\tauri.cmd build --no-bundle
Set-Location ..\..
```

可执行文件位于 `target\release\ssh-forward-desktop.exe`，可直接启动：

```powershell
.\target\release\ssh-forward-desktop.exe
```

不要直接双击 `target\debug\ssh-forward-desktop.exe`，它是开发构建，要求 `http://localhost:1420` 上的 Vite 服务正在运行。需要热更新开发时，请使用上一节的 `tauri.cmd dev` 命令。

若要生成 Windows 安装包，在 `apps\desktop` 目录执行：

```powershell
.\node_modules\.bin\tauri.cmd build
```

Tauri 会先运行 `npm run build`，再生成安装包。输出位置由 Tauri 决定，通常位于 `target\release\bundle\` 下；打包前请确认 Windows 的 WebView2 Runtime 和 MSVC 工具链可用。

桌面端默认读取项目根目录的 `config.json`，也可以通过界面顶部的“配置文件”输入框切换。当前 GUI 不会启动或显示虚假的 Tunnel 运行状态；完整的后台运行时、连接复用与自动重连仍在后续实现范围内。

GUI 中可以新建、编辑、启动、停止或删除 Local Tunnel。启动后状态由当前 GUI 进程管理；退出 GUI 会停止该 GUI 启动的 OpenSSH 子进程。认证失败、Host Key 拒绝、网络连接失败或端口冲突时，Tunnel 卡片会显示错误状态和错误信息。

## 检查与测试

在项目根目录执行 Rust 检查：

```powershell
$env:CARGO_HOME = "$PWD\.cargo"
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

在 `apps\desktop` 目录执行前端检查与生产构建：

```powershell
npm run check
npm run build
```

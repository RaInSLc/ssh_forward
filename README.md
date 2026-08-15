# SSH Forward

SSH Forward 是一个桌面端 SSH 本地端口转发工具。通过图形界面保存服务器和 Tunnel 配置，启动后将本机端口安全地转发到远端服务。

## 下载

前往 [Releases](https://github.com/RaInSLc/ssh_forward/releases) 下载最新版本。

| 平台 | 安装版 | 便携版 |
| --- | --- | --- |
| Windows x64 | `.msi` 或 `-setup.exe` | `-x64-portable.zip` |
| macOS Intel | `.dmg` | `-macos-x64-portable.zip` |
| macOS Apple Silicon | `.dmg` | `-macos-aarch64-portable.zip` |

安装版适合长期使用，会由系统安装程序完成安装。便携版解压后即可运行，不写入系统安装目录。

Windows 用户可任选 MSI 或 Setup EXE。Apple Silicon 对应 M 系列芯片；较早的 Intel Mac 请选择 macOS x64 包。

## 快速开始

首次使用按以下顺序操作：

1. 启动 SSH Forward。
2. 在左侧点击“添加服务器”。
3. 填写服务器名称、SSH 地址、SSH 端口和用户名，选择认证方式后保存。
4. 点击右上角“新建 Tunnel”。
5. 选择服务器，填写本地地址和端口，以及远端地址和端口后保存。
6. 在 Tunnel 卡片点击“启动”。状态显示“运行中”后即可通过本地地址访问远端服务。

例如，将本地 `127.0.0.1:12395` 转发到远端 `127.0.0.1:80` 后，在浏览器中访问：

```text
http://127.0.0.1:12395
```

## 添加服务器

左侧服务器列表显示已保存的服务器。点击已有服务器可编辑；点击“添加服务器”可创建新服务器。

需要填写：

- 名称：用于在 Tunnel 中选择服务器。
- 服务器地址：SSH 主机名或 IP 地址。
- SSH 端口：通常为 `22`。
- 用户名：远端 SSH 用户名。
- 认证方式：SSH Agent、私钥或密码。

### 认证方式

| 方式 | 使用方法 |
| --- | --- |
| SSH Agent | 使用系统已加载的 SSH 密钥。 |
| 私钥 | 填写本机私钥文件路径。 |
| 密码 | 输入 SSH 密码后保存。Windows 会加密保存密码。 |

Windows 密码配置只能在保存该配置的同一 Windows 用户和同一设备上使用。macOS 当前请使用 SSH Agent 或私钥认证。

首次连接未知服务器时，应用会保存该服务器的 Host Key。后续连接会继续校验；如果服务器 Host Key 变化，Tunnel 会拒绝启动。

## 创建 Tunnel

点击“新建 Tunnel”，然后填写：

- 名称：便于识别该转发，例如 `web`、`database`。
- 服务器：选择刚刚添加的 SSH 服务器。
- 本地地址与端口：本机监听地址。通常使用 `127.0.0.1`。
- 远端地址与端口：通过 SSH 服务器可访问的目标服务。
- 启动成功后自动打开浏览器：适用于 HTTP 服务，按需勾选。

Tunnel 卡片会显示：

```text
本地地址:端口 → 远端地址:端口
```

为避免把服务暴露到局域网，本地地址应使用 `127.0.0.1` 或 `localhost`。

## 使用 Tunnel

每个 Tunnel 卡片提供以下操作：

- 启动：建立 SSH 本地转发。
- 停止：关闭该转发。
- 打开浏览器：在 Tunnel 运行中，用系统默认浏览器打开本地 HTTP 地址。
- 编辑：修改服务器、地址、端口或自动打开浏览器选项。编辑运行中的 Tunnel 前请先停止它。

状态含义：

| 状态 | 含义 |
| --- | --- |
| 已停止 | 未建立转发。 |
| 运行中 | 本地端口已由 SSH Tunnel 接管。 |
| 错误 | 启动失败，请查看卡片显示的错误信息。 |

常见启动失败原因包括：本地端口已被占用、服务器不可达、认证失败、Host Key 不匹配或远端服务地址不可访问。

退出 SSH Forward 会停止由当前程序启动的 Tunnel。

## 浏览器与主题

- Tunnel 运行中可点击“打开浏览器”。
- 创建或编辑 Tunnel 时，勾选“启动成功后自动打开浏览器”，每次启动成功后会自动访问本地 HTTP 地址。
- 点击右上角“夜间模式”或“日间模式”切换界面主题，选择会在当前设备保留。

## 配置文件

- 便携版默认在程序工作目录使用 `config.json` 保存服务器与 Tunnel 配置。
- Windows 安装版（MSI / Setup EXE）在系统应用配置目录保存配置文件，路径为：%APPDATA%\com.sshforward.desktop\config.json。
- 密码认证在 Windows 上使用 DPAPI 进行加密保存。如果将 `config.json` 复制/迁移到其他设备或更换系统用户，解密密码将不可用，请在目标设备上重新编辑服务器并重新保存密码。
- 请妥善保管 `config.json` 文件，不要提交到 Git 仓库或分享给他人。
- 建议在修改大量 Tunnel 前备份 `config.json`。

## 命令行工具 (CLI)

除了桌面客户端，项目还包含命令行工具 `ssh-forward`（位于 `apps/cli`），方便在终端下管理和启动端口转发。

```bash
# 查看帮助
ssh-forward --help

# 列出已保存的服务器与 Tunnel
ssh-forward host list
ssh-forward tunnel list

# 添加服务器
ssh-forward host add my-server --host 192.168.1.100 --user root --port 22 --key ~/.ssh/id_rsa

# 添加 Tunnel
ssh-forward tunnel add web-tunnel --host my-server --local 127.0.0.1:18888 --remote 127.0.0.1:80

# 启动转发
ssh-forward start web-tunnel

# 删除配置
ssh-forward tunnel remove web-tunnel
ssh-forward host remove my-server
```

## 系统要求

- Windows：需要系统 OpenSSH；通常随 Windows 可选功能提供。
- Windows：需要 WebView2 Runtime，现代 Windows 通常已内置。
- macOS：使用系统自带 OpenSSH。

macOS 应用当前未进行 Apple Developer 签名和公证。首次打开时如被系统拦截，请在“系统设置 - 隐私与安全性”中确认打开。

## 自动发布

推送 `v*` 格式的 Git 标签会触发 GitHub Actions，在 Windows、macOS Intel 和 macOS Apple Silicon 上构建并发布安装版与便携版资产。

发布完成后可在 [Releases](https://github.com/RaInSLc/ssh_forward/releases) 下载对应文件。


#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

#[cfg(windows)]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use ssh_forward_config::{
    AuthType, Config, ConfigLock, Endpoint, Host, HostKeyPolicy, Settings, Tunnel, load, validate,
};
use ssh_forward_core::{
    AuthInput,
    HostDraft,
    RuntimePaths,
    TunnelDraft,
    // 运行期路径统一由 core 推导：壳层各自实现过一次，CLI 因此漏传而污染 ~/.ssh/known_hosts。
    known_hosts_path,
    remove_host,
    remove_tunnel,
    // 与下方 #[tauri::command] fn start_tunnel 同名，必须重命名导入，否则报 E0255。
    start_tunnel as start_tunnel_core,
    tunnel_log_path,
    upsert_host,
    upsert_tunnel,
};
use ssh_forward_ssh::{OpenSshForward, rotated_log_path};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, State};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
    UI::Shell::ShellExecuteW,
    UI::WindowsAndMessaging::SW_SHOWNORMAL,
};

/// 诊断信息回传给界面时保留的最大行数。
const DIAGNOSTIC_LINE_LIMIT: usize = 200;

/// 读取诊断日志时从文件尾部读取的初始字节窗口。
///
/// 需足够覆盖 [`DIAGNOSTIC_LINE_LIMIT`] 行（200 行 × 约 320 B ≈ 64 KB）。
/// 行数不足时会按 [`LOG_TAIL_WINDOW_GROWTH`] 倍扩张，因此这里只是「一次命中」的常见值。
const LOG_TAIL_READ_BYTES: u64 = 64 * 1024;

/// 尾部窗口行数不足时的扩张倍数。
const LOG_TAIL_WINDOW_GROWTH: u64 = 4;

/// 允许交给系统默认浏览器打开的 URL scheme 白名单。
///
/// 本模块自己构造的 URL 固定用 `http`，此白名单是**防御性**的：它把
/// 「不得打开任意 scheme」这一不变量写成可测试的断言，避免后续改动
/// （例如 D5 放宽绑定地址、或允许用户传入 URL）静默引入 `file:` / `javascript:` 等入口。
const BROWSER_URL_SCHEMES: [&str; 2] = ["http", "https"];

struct AppState {
    config_path: Mutex<PathBuf>,
    forwards: Mutex<HashMap<String, OpenSshForward>>,
    statuses: Mutex<HashMap<String, TunnelStatus>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TunnelStatus {
    state: String,
    message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    path: String,
    version: String,
    platform: String,
    /// 密码认证依赖 Windows DPAPI，其它平台需要改用 SSH Agent 或私钥。
    supports_password_auth: bool,
    known_hosts_path: String,
    config: Config,
    statuses: HashMap<String, TunnelStatus>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostInput {
    name: String,
    hostname: String,
    port: u16,
    username: String,
    auth_type: AuthType,
    private_key: Option<String>,
    password: Option<String>,
    jump_host_id: Option<String>,
    proxy_command: Option<String>,
    identities_only: Option<bool>,
    certificate_file: Option<String>,
    compression: Option<bool>,
    custom_options: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TunnelInput {
    name: String,
    host_name: String,
    #[serde(default = "default_tunnel_type")]
    kind: ssh_forward_config::TunnelType,
    local_host: String,
    local_port: u16,
    remote_host: Option<String>,
    remote_port: Option<u16>,
    gateway_ports: Option<bool>,
    custom_options: Option<Vec<String>>,
    auto_open_browser: bool,
}

fn default_tunnel_type() -> ssh_forward_config::TunnelType {
    ssh_forward_config::TunnelType::Local
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsInput {
    host_key_policy: HostKeyPolicy,
    connect_timeout_seconds: u16,
    server_alive_interval_seconds: u16,
    server_alive_count_max: u16,
    tcp_keep_alive: bool,
    compression: bool,
}

fn config_path(state: &AppState) -> Result<PathBuf, String> {
    state
        .config_path
        .lock()
        .map(|path| path.clone())
        .map_err(|_| "配置路径锁不可用".into())
}

#[cfg(debug_assertions)]
fn default_config_path(_app: &AppHandle) -> Result<PathBuf, String> {
    // Keep local development data next to the workspace for easy inspection.
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("config.json"))
}

#[cfg(not(debug_assertions))]
fn default_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|path| path.join("config.json"))
        .map_err(|error| format!("无法定位应用配置目录：{error}"))
}

// 应用私有数据目录、`known_hosts` 与隧道日志路径的推导已下沉到
// `ssh_forward_core`（见 `runtime` 模块），此处只导入使用。
// 原先桌面壳层与 CLI 各写一份，CLI 那份漏传给了 OpenSSH，导致它退回默认行为、
// 把信任记录写进用户主目录的 `~/.ssh/known_hosts`。

fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// 把服务器的 Host Key 预登记到应用私有 known_hosts。
///
/// 仅在 `accept_new` 策略下调用。失败不阻断启动，交由主 OpenSSH 连接处理。
fn prefetch_host_key(host: &Host, known_hosts: &Path) -> Result<(), String> {
    let lookup_name = if host.port == 22 {
        host.hostname.clone()
    } else {
        format!("[{}]:{}", host.hostname, host.port)
    };

    let mut lookup_cmd = Command::new("ssh-keygen");
    lookup_cmd
        .args(["-F", &lookup_name, "-f"])
        .arg(known_hosts)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    lookup_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    if lookup_cmd.status().is_ok_and(|status| status.success()) {
        return Ok(());
    }

    let mut scan_cmd = Command::new("ssh-keyscan");
    scan_cmd
        .args(["-T", "5", "-p", &host.port.to_string(), &host.hostname])
        .stdin(Stdio::null());
    #[cfg(windows)]
    scan_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let scan = match scan_cmd.output() {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => output,
        _ => {
            // 网络受限或协议特殊时探测不到，不阻断启动。
            return Ok(());
        }
    };

    let Some(parent) = known_hosts.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("无法创建 known_hosts 目录：{error}"))?;

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(known_hosts)
        .map_err(|error| format!("无法写入 {}：{error}", known_hosts.display()))?;
    file.write_all(&scan.stdout)
        .map_err(|error| format!("无法写入 {}：{error}", known_hosts.display()))?;
    if !scan.stdout.ends_with(b"\n") {
        let _ = file.write_all(b"\n");
    }
    Ok(())
}

fn local_browser_url(tunnel: &Tunnel) -> Result<String, String> {
    if tunnel.local.host != "127.0.0.1" && tunnel.local.host != "localhost" {
        return Err("仅支持用浏览器打开本地绑定地址".into());
    }
    Ok(format!(
        "http://{}:{}",
        tunnel.local.host, tunnel.local.port
    ))
}

/// 校验 URL 的 scheme 在白名单内。
///
/// 与「调用方自己拼 URL」这层约定不同，这是**可测试的显式断言**：即使将来 URL 的来源
/// 变得更宽（用户输入、配置字段），也不会把任意 scheme 交给系统 shell。
fn ensure_openable_url(url: &str) -> Result<(), String> {
    let Some((scheme, _)) = url.split_once("://") else {
        return Err("URL 缺少 scheme，已拒绝打开".into());
    };
    if !BROWSER_URL_SCHEMES
        .iter()
        .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
    {
        return Err(format!("不支持用浏览器打开 {scheme} 链接"));
    }
    Ok(())
}

/// 用系统默认浏览器打开 URL。
///
/// 刻意**不用 `cmd /C start`**：`cmd.exe` 有自己的解析规则，Rust 的标准 argv 引号机制
/// 对它不完全生效（`&`、`^`、`|` 不触发引号，会被 cmd 当作命令分隔符），
/// 于是「URL 不可注入」这一安全性只能依赖调用点的 host 白名单。
/// `ShellExecuteW` 直接把 URL 交给 shell 的「打开」动词，**不存在命令行解析层**。
#[cfg(windows)]
fn open_url_with_system(url: &str) -> Result<(), String> {
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let operation = wide("open");
    let target = wide(url);
    // SAFETY：两个宽字符串在本函数栈上存活到调用结束；其余参数按文档传空。
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;

    // ShellExecuteW 的返回值**不是** GetLastError 错误码：<= 32 表示失败。
    if result <= 32 {
        return Err(format!(
            "无法打开系统默认浏览器（ShellExecuteW 返回 {result}）"
        ));
    }
    Ok(())
}

fn open_in_browser(tunnel: &Tunnel) -> Result<(), String> {
    let url = local_browser_url(tunnel)?;
    ensure_openable_url(&url)?;
    #[cfg(windows)]
    open_url_with_system(&url)?;
    #[cfg(target_os = "macos")]
    Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|error| format!("无法打开系统默认浏览器：{error}"))?;
    #[cfg(not(any(windows, target_os = "macos")))]
    return Err("当前平台尚不支持打开系统默认浏览器".into());
    Ok(())
}

#[cfg(windows)]
fn protect_password(password: &str) -> Result<String, String> {
    let mut input = password.as_bytes().to_vec();
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: input.len().try_into().map_err(|_| "密码长度无效")?,
        pbData: input.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let success = unsafe {
        CryptProtectData(
            &input_blob,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 {
        return Err("Windows DPAPI 无法加密密码".into());
    }
    let encrypted =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(STANDARD.encode(encrypted))
}

#[cfg(not(windows))]
fn protect_password(_password: &str) -> Result<String, String> {
    Err("当前版本仅支持 Windows 的密码认证；请使用 SSH Agent 或私钥".into())
}

#[cfg(windows)]
fn unprotect_password(value: &str) -> Result<String, String> {
    let mut encrypted = STANDARD.decode(value).map_err(|_| "密码密文格式无效")?;
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len().try_into().map_err(|_| "密码密文长度无效")?,
        pbData: encrypted.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let success = unsafe {
        CryptUnprotectData(
            &input_blob,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if success == 0 {
        return Err("无法解密密码：该配置只能由保存密码的 Windows 用户在原机器上使用".into());
    }
    let password =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    String::from_utf8(password).map_err(|_| "解密后的密码不是有效 UTF-8".into())
}

#[cfg(not(windows))]
fn unprotect_password(_value: &str) -> Result<String, String> {
    Err("当前版本仅支持 Windows 的密码认证；请使用 SSH Agent 或私钥".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::{protect_password, unprotect_password};

    #[test]
    fn dpapi_round_trip_does_not_keep_plaintext_in_ciphertext() {
        let plaintext = "test-password-123";
        let encrypted = protect_password(plaintext).unwrap();
        assert_ne!(encrypted, plaintext);
        assert_eq!(unprotect_password(&encrypted).unwrap(), plaintext);
    }
}

/// 把界面输入转换为 core 的写入意图。
///
/// 密码只在用户真正输入了新值时加密；留空表示沿用已保存的密文，
/// 因此编辑其它字段不再需要重新输入密码。
fn host_draft(input: HostInput) -> Result<HostDraft, String> {
    let auth = match input.auth_type {
        AuthType::SshAgent => AuthInput::SshAgent,
        AuthType::PrivateKey => {
            let path = input
                .private_key
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or("私钥认证需要提供私钥路径")?;
            AuthInput::PrivateKey { path }
        }
        AuthType::Password => {
            let encrypted_password = input
                .password
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(protect_password)
                .transpose()?;
            AuthInput::Password { encrypted_password }
        }
    };

    Ok(HostDraft {
        name: input.name,
        hostname: input.hostname,
        port: input.port,
        username: input.username,
        auth,
        jump_host_id: input.jump_host_id,
        proxy_command: input.proxy_command,
        identities_only: input.identities_only,
        certificate_file: input.certificate_file,
        compression: input.compression,
        custom_options: input.custom_options.unwrap_or_default(),
        enabled: true,
    })
}

fn tunnel_draft(input: TunnelInput) -> Result<TunnelDraft, String> {
    let remote = match input.kind {
        ssh_forward_config::TunnelType::Dynamic => None,
        ssh_forward_config::TunnelType::Local | ssh_forward_config::TunnelType::Remote => {
            let remote_host = input.remote_host.ok_or("远端目标主机不能为空")?;
            let remote_port = input.remote_port.ok_or("远端目标端口不能为空")?;
            Some(Endpoint {
                host: remote_host,
                port: remote_port,
            })
        }
    };

    Ok(TunnelDraft {
        name: input.name,
        host_name: input.host_name,
        kind: input.kind,
        local: Endpoint {
            host: input.local_host,
            port: input.local_port,
        },
        remote,
        gateway_ports: input.gateway_ports.unwrap_or(false),
        custom_options: input.custom_options.unwrap_or_default(),
        auto_open_browser: input.auto_open_browser,
    })
}

/// OpenSSH 退出且没有可用 stderr 时的兜底提示（按认证类型给出排查方向）。
fn auth_hint(config: &Config, tunnel_name: &str) -> &'static str {
    config
        .tunnels
        .iter()
        .find(|tunnel| tunnel.name == tunnel_name)
        .and_then(|tunnel| config.hosts.iter().find(|host| host.id == tunnel.host_id))
        .map(|host| match host.auth.kind {
            AuthType::SshAgent => {
                "OpenSSH 进程已退出：当前服务器为【SSH Agent】认证。若本机未开启 ssh-agent 服务或未添加密钥将导致连接失败。建议在左侧编辑服务器切换为【密码】或【私钥】认证。"
            }
            AuthType::Password => {
                "OpenSSH 进程已退出：请检查服务器密码是否正确、用户名及端口是否可达。"
            }
            AuthType::PrivateKey => {
                "OpenSSH 进程已退出：请检查私钥文件路径是否存在、格式权限是否正确。"
            }
        })
        .unwrap_or("OpenSSH 进程已退出；请检查认证、Host Key 或网络连接")
}

/// 优先展示 OpenSSH 的真实输出，只有在拿不到输出时才退回猜测式提示。
fn describe_exit(config: &Config, tunnel_name: &str, stderr_tail: &[String]) -> String {
    if stderr_tail.is_empty() {
        return auth_hint(config, tunnel_name).into();
    }
    let recent = stderr_tail
        .iter()
        .rev()
        .take(8)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    format!("OpenSSH 已退出，原始输出：\n{recent}")
}

/// 从日志文件尾部读取最多 [`DIAGNOSTIC_LINE_LIMIT`] 行。
///
/// 刻意**不整文件读入**：诊断日志可达数 MB，而界面只需要最后若干行。
/// 做法是从尾部取一个字节窗口，若行数不足则按 [`LOG_TAIL_WINDOW_GROWTH`] 倍扩张，
/// 直到够行或已到达文件开头——这样既避免了整文件读入，又保持了
/// 「返回最后 200 行」这一契约不变。
fn read_log_tail(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut window = LOG_TAIL_READ_BYTES;

    loop {
        let start = len.saturating_sub(window);
        let mut reader = std::io::BufReader::new(&file);
        if start > 0 {
            use std::io::{Seek, SeekFrom};
            reader.seek(SeekFrom::Start(start)).ok()?;
        }
        let mut buffer = Vec::new();
        {
            use std::io::Read;
            reader.read_to_end(&mut buffer).ok()?;
        }

        // 从中间截断时，首个片段既可能是半行，也可能落在多字节字符中间；
        // 统一从第一个换行之后开始，两个问题一并解决。
        let slice = if start > 0 {
            match buffer.iter().position(|byte| *byte == b'\n') {
                Some(index) => &buffer[index + 1..],
                None => &buffer[..],
            }
        } else {
            &buffer[..]
        };

        let content = String::from_utf8_lossy(slice);
        let lines: Vec<&str> = content.lines().collect();
        if start == 0 || lines.len() >= DIAGNOSTIC_LINE_LIMIT {
            let begin = lines.len().saturating_sub(DIAGNOSTIC_LINE_LIMIT);
            return Some(lines[begin..].join("\n"));
        }
        window = window.saturating_mul(LOG_TAIL_WINDOW_GROWTH);
    }
}

/// 转发进程的最小探针接口。
///
/// `AppState.forwards` 持有具体类型 `OpenSshForward`，其构造需要真实启动一个 OpenSSH
/// 子进程，因此命令层的状态回收逻辑在普通单元测试中无法构造。把「查询进程是否存活」
/// 抽象成 trait 后，[`reconcile_forwards`] 可以用假实现覆盖，B1 的语义才能被测试锁住。
trait ForwardProbe {
    fn probe_running(&mut self) -> Result<bool, String>;
    fn probe_stderr_tail(&self) -> Vec<String>;
}

impl ForwardProbe for OpenSshForward {
    fn probe_running(&mut self) -> Result<bool, String> {
        // 完全限定语法：避免与同名 trait 方法产生歧义。
        <OpenSshForward>::is_running(self).map_err(|error| error.to_string())
    }

    fn probe_stderr_tail(&self) -> Vec<String> {
        <OpenSshForward>::stderr_tail(self)
    }
}

/// 探测全部转发进程，回收已退出的条目。
///
/// 返回「需要写回状态表的变更条目」而不是整张状态表：调用方只应用这些条目，
/// 不得整体覆盖，否则会丢掉 `start_tunnel` 在探测期间写入的 `starting` / `running`。
fn reconcile_forwards<F: ForwardProbe>(
    forwards: &mut HashMap<String, F>,
    config: &Config,
) -> Vec<(String, TunnelStatus)> {
    let mut updates = Vec::new();
    for (name, forward) in forwards.iter_mut() {
        let running = forward.probe_running();
        let message = match running {
            Ok(true) => continue,
            Ok(false) => describe_exit(config, name, &forward.probe_stderr_tail()),
            Err(error) => error,
        };
        updates.push((
            name.clone(),
            TunnelStatus {
                state: "error".into(),
                message: Some(message),
            },
        ));
    }
    for (name, _) in &updates {
        forwards.remove(name);
    }
    updates
}

/// 把探测结果写回状态表。
///
/// 刻意只对变更条目做 `insert`，绝不整体赋值：`snapshot` 在克隆状态表与写回之间要读取
/// 配置并探测进程，期间 `start_tunnel` 可能写入 `starting` / `running`；整体覆盖会把这些
/// 写入抹掉，而运行中的隧道又不会被重新写回 `running`，于是前端永久回退为 `stopped`
/// 且无法自愈。
fn apply_status_updates(
    store: &mut HashMap<String, TunnelStatus>,
    updates: Vec<(String, TunnelStatus)>,
) {
    for (name, status) in updates {
        store.insert(name, status);
    }
}

fn snapshot(state: &AppState) -> Result<Snapshot, String> {
    let path = config_path(state)?;
    let config = load(&path).map_err(|error| error.to_string())?;
    let updates = {
        let mut forwards = state.forwards.lock().map_err(|_| "Tunnel 运行时锁不可用")?;
        reconcile_forwards(&mut forwards, &config)
    };
    // 变更应用与响应克隆在同一临界区内完成，保证返回的状态表与实际存储一致。
    let statuses = {
        let mut store = state.statuses.lock().map_err(|_| "Tunnel 状态锁不可用")?;
        apply_status_updates(&mut store, updates);
        store.clone()
    };
    Ok(Snapshot {
        path: path.display().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: platform_name().into(),
        supports_password_auth: cfg!(windows),
        known_hosts_path: known_hosts_path(&path).display().to_string(),
        config,
        statuses,
    })
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    snapshot(&state)
}

#[tauri::command]
fn get_available_port(host: Option<String>) -> Result<u16, String> {
    let host = host.unwrap_or_else(|| "127.0.0.1".into());
    let listener = std::net::TcpListener::bind(format!("{host}:0"))
        .map_err(|e| format!("无法分配空闲端口: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("无法获取端口号: {e}"))?
        .port();
    drop(listener);
    Ok(port)
}

#[tauri::command]
fn set_config_path(path: String, state: State<'_, AppState>) -> Result<Snapshot, String> {
    let candidate = PathBuf::from(path.trim());
    if candidate.as_os_str().is_empty() {
        return Err("配置文件路径不能为空".into());
    }
    load(&candidate).map_err(|error| error.to_string())?;
    *state.config_path.lock().map_err(|_| "配置路径锁不可用")? = candidate;
    snapshot(&state)
}

#[tauri::command]
fn validate_config(state: State<'_, AppState>) -> Result<String, String> {
    validate(&snapshot(&state)?.config).map_err(|error| error.to_string())?;
    Ok("配置校验通过".into())
}

#[tauri::command]
fn save_settings(input: SettingsInput, state: State<'_, AppState>) -> Result<Settings, String> {
    let path = config_path(&state)?;
    let _guard = ConfigLock::acquire(&path).map_err(|error| error.to_string())?;
    let mut config = load(&path).map_err(|error| error.to_string())?;
    config.settings = Settings {
        host_key_policy: input.host_key_policy,
        connect_timeout_seconds: input.connect_timeout_seconds,
        server_alive_interval_seconds: input.server_alive_interval_seconds,
        server_alive_count_max: input.server_alive_count_max,
        tcp_keep_alive: input.tcp_keep_alive,
        compression: input.compression,
    };
    validate(&config).map_err(|error| error.to_string())?;
    ssh_forward_config::save(&path, &config).map_err(|error| error.to_string())?;
    Ok(config.settings)
}

#[tauri::command]
fn create_host(input: HostInput, state: State<'_, AppState>) -> Result<Host, String> {
    let draft = host_draft(input)?;
    upsert_host(&config_path(&state)?, None, draft).map_err(|error| error.to_string())
}

#[tauri::command]
fn edit_host(
    original_name: String,
    input: HostInput,
    state: State<'_, AppState>,
) -> Result<Host, String> {
    let draft = host_draft(input)?;
    upsert_host(&config_path(&state)?, Some(&original_name), draft)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_host(name: String, state: State<'_, AppState>) -> Result<(), String> {
    remove_host(&config_path(&state)?, &name).map_err(|error| error.to_string())
}

#[tauri::command]
fn create_tunnel(input: TunnelInput, state: State<'_, AppState>) -> Result<Tunnel, String> {
    let draft = tunnel_draft(input)?;
    upsert_tunnel(&config_path(&state)?, None, draft).map_err(|error| error.to_string())
}

#[tauri::command]
fn edit_tunnel(
    original_name: String,
    input: TunnelInput,
    state: State<'_, AppState>,
) -> Result<Tunnel, String> {
    if state
        .forwards
        .lock()
        .map_err(|_| "Tunnel 运行时锁不可用")?
        .contains_key(&original_name)
    {
        return Err("请先停止 Tunnel 再编辑".into());
    }
    let draft = tunnel_draft(input)?;
    upsert_tunnel(&config_path(&state)?, Some(&original_name), draft)
        .map_err(|error| error.to_string())
}

/// 删除某条隧道的诊断日志（当前文件与轮转后的旧文件）。
///
/// 刻意**静默忽略失败**：日志属于诊断附属物，清理不成功不应让「删除隧道」这一
/// 用户操作整体失败。同时只删除由 [`tunnel_log_path`] 推导出的两个路径，
/// 不触碰配置目录中的其他内容。
fn remove_tunnel_logs(config_path: &Path, tunnel_id: &str) {
    let path = tunnel_log_path(config_path, tunnel_id);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(rotated_log_path(&path));
}

#[tauri::command]
fn delete_tunnel(name: String, state: State<'_, AppState>) -> Result<(), String> {
    stop_tunnel(name.clone(), state.clone())?;
    let path = config_path(&state)?;
    // 必须在 remove_tunnel 之前取 id：删除后配置里就查不到这条隧道，
    // 而日志文件名由 id 决定（C3），届时无法再推导出日志路径。
    let tunnel_id = load(&path)
        .map_err(|error| error.to_string())?
        .tunnels
        .iter()
        .find(|tunnel| tunnel.name == name)
        .map(|tunnel| tunnel.id.clone());
    remove_tunnel(&path, &name).map_err(|error| error.to_string())?;
    if let Some(tunnel_id) = tunnel_id {
        remove_tunnel_logs(&path, &tunnel_id);
    }
    Ok(())
}

#[tauri::command]
fn start_tunnel(name: String, state: State<'_, AppState>) -> Result<(), String> {
    if state
        .forwards
        .lock()
        .map_err(|_| "Tunnel 运行时锁不可用")?
        .contains_key(&name)
    {
        return Ok(());
    }
    state
        .statuses
        .lock()
        .map_err(|_| "Tunnel 状态锁不可用")?
        .insert(
            name.clone(),
            TunnelStatus {
                state: "starting".into(),
                message: None,
            },
        );

    let run_start = || -> Result<(), String> {
        let path = config_path(&state)?;
        let config = snapshot(&state)?.config;
        let tunnel = config
            .tunnels
            .iter()
            .find(|tunnel| tunnel.name == name)
            .ok_or("未找到 Tunnel")?;
        let host = config
            .hosts
            .iter()
            .find(|host| host.id == tunnel.host_id)
            .ok_or("未找到 Tunnel 对应的服务器")?;

        // 运行期路径统一由 core 推导，壳层只需补密码，结构上无法再遗漏。
        let paths = RuntimePaths::for_tunnel(&path, &tunnel.id);
        if config.settings.host_key_policy.should_prefetch() {
            prefetch_host_key(host, paths.known_hosts())?;
        }

        let password = match host.auth.kind {
            AuthType::Password => unprotect_password(
                host.auth
                    .encrypted_password
                    .as_deref()
                    .ok_or("该服务器使用旧密码配置；请编辑服务器并重新输入密码以迁移到加密配置")?,
            )
            .map(Some),
            _ => Ok(None),
        }?;

        let forward = start_tunnel_core(&config, &name, paths.options(password.as_deref()))
            .map_err(|error| error.to_string())?;

        state
            .forwards
            .lock()
            .map_err(|_| "Tunnel 运行时锁不可用")?
            .insert(name.clone(), forward);
        state
            .statuses
            .lock()
            .map_err(|_| "Tunnel 状态锁不可用")?
            .insert(
                name.clone(),
                TunnelStatus {
                    state: "running".into(),
                    message: None,
                },
            );
        if tunnel.auto_open_browser {
            let _ = open_in_browser(tunnel);
        }
        Ok(())
    };

    match run_start() {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = state.statuses.lock().map(|mut statuses| {
                statuses.insert(
                    name,
                    TunnelStatus {
                        state: "error".into(),
                        message: Some(error.clone()),
                    },
                );
            });
            Err(error)
        }
    }
}

/// 返回某个 Tunnel 的诊断信息：优先取内存中的 OpenSSH 输出，其次读落盘日志。
#[tauri::command]
fn get_tunnel_diagnostics(name: String, state: State<'_, AppState>) -> Result<String, String> {
    let path = config_path(&state)?;
    if let Ok(forwards) = state.forwards.lock()
        && let Some(forward) = forwards.get(&name)
    {
        let text = forward.diagnostics_text();
        if !text.is_empty() {
            return Ok(text);
        }
        // 内存无输出时，直接沿用该进程启动时真正写入的日志文件。
        if let Some(log_path) = forward.log_path() {
            return read_log_tail(log_path).ok_or_else(|| {
                format!(
                    "暂无诊断信息（日志文件 {} 不存在或为空）",
                    log_path.display()
                )
            });
        }
    }

    // 隧道未运行：按配置中的隧道 id 推导日志路径，与 start_tunnel 的命名保持一致。
    let config = load(&path).map_err(|error| error.to_string())?;
    let tunnel = config
        .tunnels
        .iter()
        .find(|tunnel| tunnel.name == name)
        .ok_or("未找到 Tunnel")?;
    let log_path = tunnel_log_path(&path, &tunnel.id);
    read_log_tail(&log_path).ok_or_else(|| {
        format!(
            "暂无诊断信息（日志文件 {} 不存在或为空）",
            log_path.display()
        )
    })
}

#[tauri::command]
fn open_tunnel_in_browser(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let config = snapshot(&state)?.config;
    let tunnel = config
        .tunnels
        .iter()
        .find(|tunnel| tunnel.name == name)
        .ok_or("未找到 Tunnel")?;
    open_in_browser(tunnel)
}

#[tauri::command]
fn stop_tunnel(name: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(mut forward) = state
        .forwards
        .lock()
        .map_err(|_| "Tunnel 运行时锁不可用")?
        .remove(&name)
    {
        forward.stop().map_err(|error| error.to_string())?;
    }
    state
        .statuses
        .lock()
        .map_err(|_| "Tunnel 状态锁不可用")?
        .insert(
            name,
            TunnelStatus {
                state: "stopped".into(),
                message: None,
            },
        );
    Ok(())
}

fn cleanup_forwards(state: &AppState) {
    if let Ok(mut forwards) = state.forwards.lock() {
        for (_, mut forward) in forwards.drain() {
            let _ = forward.stop();
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _cwd| {
                // 第二个实例不再各自持有转发，而是把已有窗口带到前台。
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            },
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            app.manage(AppState {
                config_path: Mutex::new(
                    default_config_path(app.handle()).map_err(std::io::Error::other)?,
                ),
                forwards: Mutex::new(HashMap::new()),
                statuses: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) && let Some(state) = window.try_state::<AppState>()
            {
                cleanup_forwards(&state);
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_available_port,
            set_config_path,
            validate_config,
            save_settings,
            create_host,
            edit_host,
            delete_host,
            create_tunnel,
            edit_tunnel,
            delete_tunnel,
            start_tunnel,
            get_tunnel_diagnostics,
            open_tunnel_in_browser,
            stop_tunnel
        ])
        .build(tauri::generate_context!())
        .expect("error while building SSH Forward desktop application")
        .run(|app_handle, event| {
            if matches!(
                event,
                tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
            ) && let Some(state) = app_handle.try_state::<AppState>()
            {
                cleanup_forwards(&state);
            }
        });
}

/// 状态回收与日志命名的回归测试。
///
/// 这些用例之所以此前不可能存在，是因为探测逻辑直接依赖具体类型 `OpenSshForward`
/// （构造它需要真实启动 OpenSSH 子进程）。抽出 [`ForwardProbe`] 后即可用假探针覆盖。
#[cfg(test)]
mod status_pipeline_tests {
    use super::{
        Config, ForwardProbe, TunnelStatus, apply_status_updates, reconcile_forwards,
        tunnel_log_path,
    };
    // 桌面壳层只在测试里用到它，故不放进模块级导入（否则非测试构建会报未使用导入）。
    use ssh_forward_core::sanitize_file_stem;
    use std::collections::HashMap;
    use std::path::Path;

    /// 可控的假探针：无需 SSH 进程即可脚本化「存活 / 已退出 / 探测失败」三种结果。
    struct FakeForward {
        running: Result<bool, String>,
        stderr: Vec<String>,
    }

    impl FakeForward {
        fn alive() -> Self {
            Self {
                running: Ok(true),
                stderr: Vec::new(),
            }
        }

        fn exited(stderr: &[&str]) -> Self {
            Self {
                running: Ok(false),
                stderr: stderr.iter().map(|line| (*line).to_string()).collect(),
            }
        }

        fn probe_failed(message: &str) -> Self {
            Self {
                running: Err(message.to_string()),
                stderr: Vec::new(),
            }
        }
    }

    impl ForwardProbe for FakeForward {
        fn probe_running(&mut self) -> Result<bool, String> {
            self.running.clone()
        }

        fn probe_stderr_tail(&self) -> Vec<String> {
            self.stderr.clone()
        }
    }

    fn status(state: &str) -> TunnelStatus {
        TunnelStatus {
            state: state.into(),
            message: None,
        }
    }

    fn state_of<'a>(store: &'a HashMap<String, TunnelStatus>, name: &str) -> Option<&'a str> {
        store.get(name).map(|status| status.state.as_str())
    }

    /* ---------------------------------------------------------------- */
    /* B1：状态表只允许增量写回                                          */
    /* ---------------------------------------------------------------- */

    /// B1 回归：`snapshot` 克隆状态表之后、写回之前，`start_tunnel` 可能写入 `running`。
    /// 若写回是整体赋值，该条目会被抹掉，而运行中的隧道又不会被重新写回，
    /// 前端将永久显示 `stopped` 且无法自愈。
    #[test]
    fn apply_status_updates_preserves_entries_written_after_clone() {
        let mut store = HashMap::new();
        store.insert("live".to_string(), status("running"));

        apply_status_updates(&mut store, vec![("dead".to_string(), status("error"))]);

        assert_eq!(state_of(&store, "live"), Some("running"));
        assert_eq!(state_of(&store, "dead"), Some("error"));
        assert_eq!(store.len(), 2);
    }

    /// 无变更时必须原样保留状态表（旧实现在此处也会重写整张表）。
    #[test]
    fn apply_status_updates_with_no_changes_keeps_store_intact() {
        let mut store = HashMap::new();
        store.insert("live".to_string(), status("running"));

        apply_status_updates(&mut store, Vec::new());

        assert_eq!(state_of(&store, "live"), Some("running"));
        assert_eq!(store.len(), 1);
    }

    /* ---------------------------------------------------------------- */
    /* B1：探测语义                                                      */
    /* ---------------------------------------------------------------- */

    #[test]
    fn reconcile_leaves_alive_tunnels_untouched() {
        let mut forwards = HashMap::new();
        forwards.insert("alive".to_string(), FakeForward::alive());

        let updates = reconcile_forwards(&mut forwards, &Config::default());

        assert!(updates.is_empty(), "运行中的隧道不应产生状态变更");
        assert!(forwards.contains_key("alive"), "运行中的隧道不应被回收");
    }

    #[test]
    fn reconcile_marks_exited_tunnel_as_error_and_reclaims_it() {
        let mut forwards = HashMap::new();
        forwards.insert(
            "dead".to_string(),
            FakeForward::exited(&["Permission denied (publickey)."]),
        );

        let updates = reconcile_forwards(&mut forwards, &Config::default());

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, "dead");
        assert_eq!(updates[0].1.state, "error");
        assert!(
            updates[0]
                .1
                .message
                .as_deref()
                .is_some_and(|message| message.contains("Permission denied")),
            "应优先展示 OpenSSH 的真实输出"
        );
        assert!(!forwards.contains_key("dead"), "已退出的隧道应被回收");
    }

    /// 无 stderr 时退回按认证类型的排查提示，而不是空消息。
    #[test]
    fn reconcile_falls_back_to_auth_hint_when_stderr_is_empty() {
        let mut forwards = HashMap::new();
        forwards.insert("dead".to_string(), FakeForward::exited(&[]));

        let updates = reconcile_forwards(&mut forwards, &Config::default());

        assert_eq!(updates[0].1.state, "error");
        assert!(
            updates[0]
                .1
                .message
                .as_deref()
                .is_some_and(|message| message.contains("OpenSSH 进程已退出"))
        );
    }

    /// 探针自身报错同样要标记为 `error` 并回收，避免僵尸条目长期占用 `forwards`。
    #[test]
    fn reconcile_reports_probe_failure_and_reclaims_tunnel() {
        let mut forwards = HashMap::new();
        forwards.insert(
            "broken".to_string(),
            FakeForward::probe_failed("进程状态不可读"),
        );

        let updates = reconcile_forwards(&mut forwards, &Config::default());

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1.message.as_deref(), Some("进程状态不可读"));
        assert!(!forwards.contains_key("broken"));
    }

    /// 混合场景：只有真正发生变化的条目进入 updates，存活条目必须原样保留。
    #[test]
    fn reconcile_only_reports_entries_that_changed() {
        let mut forwards = HashMap::new();
        forwards.insert("alive-a".to_string(), FakeForward::alive());
        forwards.insert("alive-b".to_string(), FakeForward::alive());
        forwards.insert(
            "dead".to_string(),
            FakeForward::exited(&["Connection refused"]),
        );

        let updates = reconcile_forwards(&mut forwards, &Config::default());

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, "dead");
        assert_eq!(forwards.len(), 2);
        assert!(forwards.contains_key("alive-a"));
        assert!(forwards.contains_key("alive-b"));
    }

    /* ---------------------------------------------------------------- */
    /* C3：日志文件名以隧道 id 为准，而不是隧道名                        */
    /* ---------------------------------------------------------------- */

    /// 归一化确实会把不同隧道名折叠成同一个文件名——这正是 C3 要消除的碰撞来源。
    #[test]
    fn sanitize_collapses_distinct_tunnel_names() {
        assert_eq!(sanitize_file_stem("web api"), sanitize_file_stem("web:api"));
        assert_eq!(sanitize_file_stem("web:api"), sanitize_file_stem("web_api"));
        assert_eq!(sanitize_file_stem(""), "unnamed");
    }

    /// 改用 id 后，即使隧道名归一化后完全一致，日志路径也不会碰撞。
    #[test]
    fn tunnel_log_path_uses_id_so_names_cannot_collide() {
        let config_path = Path::new("C:/app/config.json");
        let first = tunnel_log_path(config_path, "11111111-1111-1111-1111-111111111111");
        let second = tunnel_log_path(config_path, "22222222-2222-2222-2222-222222222222");

        assert_ne!(first, second, "不同 id 必须得到不同日志文件");
        assert!(
            first
                .to_string_lossy()
                .ends_with("tunnel-11111111-1111-1111-1111-111111111111.log"),
            "id 只含十六进制字符与连字符，归一化不应改写它：{}",
            first.display()
        );
        assert!(
            first
                .parent()
                .is_some_and(|parent| parent.ends_with("logs")),
            "日志仍应落在配置目录下的 logs 子目录：{}",
            first.display()
        );
    }
}

#[cfg(test)]
mod log_lifecycle_tests {
    use super::{
        DIAGNOSTIC_LINE_LIMIT, read_log_tail, remove_tunnel_logs, rotated_log_path, tunnel_log_path,
    };
    use std::path::{Path, PathBuf};

    /// 每个用例独占的临时目录。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ssh-forward-main-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir
    }

    /// 写入 `count` 行固定形状的日志，每行 `line-<四位序号>\n`（10 字节）。
    fn write_numbered_lines(path: &Path, count: usize) {
        let content: String = (0..count)
            .map(|index| format!("line-{index:04}\n"))
            .collect();
        std::fs::write(path, content).expect("写入日志失败");
    }

    /// 文件远大于读取窗口时，只返回尾部 [`DIAGNOSTIC_LINE_LIMIT`] 行。
    #[test]
    fn reads_only_the_tail_when_the_file_exceeds_the_window() {
        let dir = temp_dir("tail-large");
        let path = dir.join("tunnel.log");
        // 10000 行 × 10 B = 100 KB，超过 64 KB 的初始窗口。
        write_numbered_lines(&path, 10_000);

        let tail = read_log_tail(&path).expect("读取尾部失败");
        let lines: Vec<&str> = tail.lines().collect();

        assert_eq!(lines.len(), DIAGNOSTIC_LINE_LIMIT, "应恰好返回行数上限");
        assert_eq!(
            lines.first().copied(),
            Some("line-9800"),
            "起点应为倒数第 200 行"
        );
        assert_eq!(
            lines.last().copied(),
            Some("line-9999"),
            "末行应为文件最后一行"
        );
    }

    /// 文件小于窗口时返回全部内容，且不因「从中间截断」而丢首行。
    #[test]
    fn returns_every_line_when_the_file_is_smaller_than_the_window() {
        let dir = temp_dir("tail-small");
        let path = dir.join("tunnel.log");
        write_numbered_lines(&path, 10);

        let tail = read_log_tail(&path).expect("读取尾部失败");
        let lines: Vec<&str> = tail.lines().collect();

        assert_eq!(lines.len(), 10);
        assert_eq!(lines.first().copied(), Some("line-0000"));
        assert_eq!(lines.last().copied(), Some("line-0009"));
    }

    /// 行很长时初始窗口装不下 200 行，必须扩张窗口直到够行。
    ///
    /// 这条用例锁住的是「窗口读取不得改变返回行数契约」。
    #[test]
    fn expands_the_window_when_lines_are_very_long() {
        let dir = temp_dir("tail-long-lines");
        let path = dir.join("tunnel.log");
        let payload = "y".repeat(995);
        let content: String = (0..300)
            .map(|index| format!("{payload}{index:04}\n"))
            .collect();
        std::fs::write(&path, content).expect("写入日志失败");

        let tail = read_log_tail(&path).expect("读取尾部失败");
        let lines: Vec<&str> = tail.lines().collect();

        assert_eq!(
            lines.len(),
            DIAGNOSTIC_LINE_LIMIT,
            "64 KB 窗口只够约 65 行，必须扩张后才能返回 200 行"
        );
        assert!(
            lines.last().expect("末行").ends_with("0299"),
            "末行应为第 299 行"
        );
    }

    /// 窗口边界落在多字节字符中间时，不得产出替换字符。
    ///
    /// 若按 `String` 直接切片，这里会因 UTF-8 边界错误而失败或产生 U+FFFD；
    /// 实现改为「按字节读取 + 丢弃首个不完整行」，两个问题一并规避。
    #[test]
    fn does_not_split_multibyte_characters_at_the_window_boundary() {
        let dir = temp_dir("tail-multibyte");
        let path = dir.join("tunnel.log");
        // 每行 12 字节：`日志`（6 B）+ `-`（1 B）+ 四位序号（4 B）+ 换行（1 B）。
        // 12 不是 65536 的因数，因此窗口边界必然落在行内，且可能落在汉字中间。
        let content: String = (0..6000)
            .map(|index| format!("日志-{index:04}\n"))
            .collect();
        std::fs::write(&path, content).expect("写入日志失败");

        let tail = read_log_tail(&path).expect("读取尾部失败");
        let lines: Vec<&str> = tail.lines().collect();

        assert!(
            !tail.contains('\u{FFFD}'),
            "出现替换字符说明切到了多字节字符中间"
        );
        assert_eq!(lines.len(), DIAGNOSTIC_LINE_LIMIT);
        assert_eq!(lines.last().copied(), Some("日志-5999"));
    }

    /// 文件不存在时返回 `None`，而不是 panic。
    #[test]
    fn missing_file_yields_none() {
        let dir = temp_dir("tail-missing");
        assert!(read_log_tail(&dir.join("nope.log")).is_none());
    }

    /// 删除隧道时同时清理当前日志与轮转后的旧日志，且不动目录中的其他文件。
    #[test]
    fn removes_both_current_and_rotated_logs() {
        let dir = temp_dir("cleanup");
        let config_path = dir.join("config.json");
        let tunnel_id = "11111111-1111-1111-1111-111111111111";
        let log = tunnel_log_path(&config_path, tunnel_id);
        std::fs::create_dir_all(log.parent().expect("日志应有父目录")).expect("创建 logs 失败");
        std::fs::write(&log, "current").expect("写入当前日志失败");
        std::fs::write(rotated_log_path(&log), "rotated").expect("写入旧日志失败");
        // 同目录下的无关文件必须保留。
        let unrelated = dir.join("known_hosts");
        std::fs::write(&unrelated, "keep-me").expect("写入无关文件失败");

        remove_tunnel_logs(&config_path, tunnel_id);

        assert!(!log.exists(), "当前日志应被删除");
        assert!(!rotated_log_path(&log).exists(), "轮转后的旧日志应被删除");
        assert!(unrelated.exists(), "无关文件不得被删除");
    }

    /// 日志本就不存在时，清理不得 panic——它必须对「删隧道」这一操作完全透明。
    #[test]
    fn removing_logs_is_silent_when_files_are_absent() {
        let dir = temp_dir("cleanup-absent");
        let config_path = dir.join("config.json");

        remove_tunnel_logs(&config_path, "no-such-tunnel");

        assert!(!tunnel_log_path(&config_path, "no-such-tunnel").exists());
    }
}

#[cfg(test)]
mod browser_url_tests {
    use super::{ensure_openable_url, local_browser_url};
    use ssh_forward_config::{Endpoint, Tunnel, TunnelType};

    fn tunnel(host: &str, port: u16) -> Tunnel {
        Tunnel {
            id: "11111111-1111-1111-1111-111111111111".into(),
            name: "web".into(),
            host_id: "host-1".into(),
            kind: TunnelType::Local,
            local: Endpoint {
                host: host.into(),
                port,
            },
            remote: Some(Endpoint {
                host: "example.test".into(),
                port: 80,
            }),
            gateway_ports: false,
            custom_options: Vec::new(),
            auto_start: false,
            auto_reconnect: true,
            auto_open_browser: false,
            enabled: true,
        }
    }

    /// scheme 白名单是「不得打开任意 scheme」这一不变量的可测试表达。
    #[test]
    fn accepts_http_and_https() {
        assert!(ensure_openable_url("http://127.0.0.1:8080").is_ok());
        assert!(ensure_openable_url("https://127.0.0.1:8080").is_ok());
        assert!(
            ensure_openable_url("HTTP://127.0.0.1:8080").is_ok(),
            "scheme 比较应忽略大小写"
        );
    }

    /// 白名单之外的一切 scheme 都必须被拒绝。
    #[test]
    fn rejects_schemes_outside_the_whitelist() {
        for url in [
            "file:///C:/Windows/System32/calc.exe",
            "ftp://127.0.0.1",
            "javascript://127.0.0.1",
            "ms-msdt://127.0.0.1",
            "vbscript://127.0.0.1",
        ] {
            assert!(
                ensure_openable_url(url).is_err(),
                "{url} 不应被允许交给系统打开"
            );
        }
    }

    /// 缺少 `://` 的输入无法判定 scheme，必须拒绝而非放行。
    #[test]
    fn rejects_urls_without_a_scheme_separator() {
        for url in ["127.0.0.1:8080", "cmd:/c calc", "", "://"] {
            assert!(
                ensure_openable_url(url).is_err(),
                "{url} 缺少 scheme，应被拒绝"
            );
        }
    }

    /// 本批**刻意未**放宽绑定地址白名单（那是 D5 的范围）。
    ///
    /// 该用例是 D5 的现状记录测试：D5 落地后它会失败，届时需连同
    /// `local_browser_url` 一起改写为新的预期行为。
    #[test]
    fn local_browser_url_still_restricts_the_bind_address() {
        for host in ["0.0.0.0", "192.168.1.10", "::", "[::1]"] {
            assert!(
                local_browser_url(&tunnel(host, 8080)).is_err(),
                "{host} 当前不应被允许用浏览器打开"
            );
        }
    }

    #[test]
    fn local_browser_url_builds_http_for_loopback() {
        assert_eq!(
            local_browser_url(&tunnel("127.0.0.1", 8080)).expect("应允许"),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            local_browser_url(&tunnel("localhost", 9090)).expect("应允许"),
            "http://localhost:9090"
        );
    }

    /// 端到端串联：`local_browser_url` 的产物必须能通过 scheme 白名单。
    #[test]
    fn loopback_url_passes_the_scheme_whitelist() {
        let url = local_browser_url(&tunnel("127.0.0.1", 8080)).expect("应允许");
        assert!(ensure_openable_url(&url).is_ok());
    }
}

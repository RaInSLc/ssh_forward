#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
};

#[cfg(windows)]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use ssh_forward_config::{Auth, AuthType, Config, Endpoint, Host, Tunnel, load, validate};
use ssh_forward_core::{remove_host, remove_tunnel, start_tunnel_with_password, update_tunnel};
use ssh_forward_ssh::OpenSshForward;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, State};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};

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
    strict_host_key_checking: bool,
    connect_timeout_seconds: u16,
    server_alive_interval_seconds: u16,
    server_alive_count_max: u16,
    tcp_keep_alive: bool,
    compression: bool,
}

use serde::Deserialize;

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

fn known_hosts_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or("无法定位当前用户主目录")?;
    Ok(PathBuf::from(home).join(".ssh").join("known_hosts"))
}

fn ensure_host_key(hostname: &str, port: u16) -> Result<(), String> {
    let lookup_name = if port == 22 {
        hostname.to_owned()
    } else {
        format!("[{hostname}]:{port}")
    };
    let mut lookup_cmd = Command::new("ssh-keygen");
    lookup_cmd
        .args(["-F", &lookup_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    lookup_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    if lookup_cmd.status().is_ok_and(|status| status.success()) {
        return Ok(());
    }

    let mut scan_cmd = Command::new("ssh-keyscan");
    scan_cmd
        .args(["-T", "5", "-p", &port.to_string(), hostname])
        .stdin(Stdio::null());
    #[cfg(windows)]
    scan_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let scan = match scan_cmd.output() {
        Ok(output) if output.status.success() && !output.stdout.is_empty() => output,
        _ => {
            // 如果 ssh-keyscan 预探测未获取到（例如网络防火墙拦截或特殊协议），不阻断启动，交由主 OpenSSH 连接处理
            return Ok(());
        }
    };

    let path = match known_hosts_path() {
        Ok(path) => path,
        Err(_) => return Ok(()),
    };
    let parent = match path.parent() {
        Some(parent) => parent,
        None => return Ok(()),
    };
    let _ = std::fs::create_dir_all(parent);
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = file.write_all(&scan.stdout);
        if !scan.stdout.ends_with(b"\n") {
            let _ = file.write_all(b"\n");
        }
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

fn open_in_browser(tunnel: &Tunnel) -> Result<(), String> {
    let url = local_browser_url(tunnel)?;
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", &url]);
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        cmd.spawn()
            .map_err(|error| format!("无法打开系统默认浏览器：{error}"))?;
    }
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

fn build_auth(input: &HostInput, existing: Option<&Host>) -> Result<Auth, String> {
    match input.auth_type {
        AuthType::SshAgent => Ok(Auth::default()),
        AuthType::PrivateKey => {
            let private_key = input
                .private_key
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or("私钥认证需要提供私钥路径")?;
            Ok(Auth {
                kind: AuthType::PrivateKey,
                private_key: Some(private_key),
                credential_id: None,
                encrypted_password: None,
            })
        }
        AuthType::Password => {
            if let Some(password) = input.password.as_deref().filter(|value| !value.is_empty()) {
                return Ok(Auth {
                    kind: AuthType::Password,
                    private_key: None,
                    credential_id: None,
                    encrypted_password: Some(protect_password(password)?),
                });
            }
            if let Some(host) = existing.filter(|host| host.auth.kind == AuthType::Password) {
                return Ok(host.auth.clone());
            }
            if existing.is_none() {
                return Err("密码认证需要输入密码".into());
            }
            Err("密码认证需要重新输入密码".into())
        }
    }
}

fn snapshot(state: &AppState) -> Result<Snapshot, String> {
    let path = config_path(state)?;
    let config = load(&path).map_err(|error| error.to_string())?;
    let mut statuses = state
        .statuses
        .lock()
        .map_err(|_| "Tunnel 状态锁不可用")?
        .clone();
    let mut forwards = state.forwards.lock().map_err(|_| "Tunnel 运行时锁不可用")?;
    let mut exited = Vec::new();
    for (name, forward) in forwards.iter_mut() {
        match forward.is_running() {
            Ok(true) => {}
            Ok(false) => {
                exited.push(name.clone());
                let error_hint = config
                    .tunnels
                    .iter()
                    .find(|t| t.name == *name)
                    .and_then(|t| config.hosts.iter().find(|h| h.id == t.host_id))
                    .map(|h| match h.auth.kind {
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
                    .unwrap_or("OpenSSH 进程已退出；请检查认证、Host Key 或网络连接");
                statuses.insert(
                    name.clone(),
                    TunnelStatus {
                        state: "error".into(),
                        message: Some(error_hint.into()),
                    },
                );
            }
            Err(error) => {
                exited.push(name.clone());
                statuses.insert(
                    name.clone(),
                    TunnelStatus {
                        state: "error".into(),
                        message: Some(error.to_string()),
                    },
                );
            }
        }
    }
    for name in exited {
        forwards.remove(&name);
    }
    drop(forwards);
    *state.statuses.lock().map_err(|_| "Tunnel 状态锁不可用")? = statuses.clone();
    Ok(Snapshot {
        path: path.display().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
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
fn save_settings(
    input: SettingsInput,
    state: State<'_, AppState>,
) -> Result<ssh_forward_config::Settings, String> {
    let path = config_path(&state)?;
    let mut config = load(&path).map_err(|e| e.to_string())?;
    config.settings = ssh_forward_config::Settings {
        strict_host_key_checking: input.strict_host_key_checking,
        connect_timeout_seconds: input.connect_timeout_seconds,
        server_alive_interval_seconds: input.server_alive_interval_seconds,
        server_alive_count_max: input.server_alive_count_max,
        tcp_keep_alive: input.tcp_keep_alive,
        compression: input.compression,
    };
    validate(&config).map_err(|e| e.to_string())?;
    ssh_forward_config::save(&path, &config).map_err(|e| e.to_string())?;
    Ok(config.settings)
}

#[tauri::command]
fn create_host(input: HostInput, state: State<'_, AppState>) -> Result<Host, String> {
    let auth = build_auth(&input, None)?;
    let path = config_path(&state)?;
    let mut config = load(&path).map_err(|e| e.to_string())?;
    if config.hosts.iter().any(|h| h.name == input.name) {
        return Err(format!("已存在名为 '{}' 的服务器", input.name));
    }
    let host = Host {
        id: uuid::Uuid::new_v4().to_string(),
        name: input.name,
        hostname: input.hostname,
        port: input.port,
        username: input.username,
        auth,
        jump_host_id: input.jump_host_id.filter(|s| !s.trim().is_empty()),
        proxy_command: input.proxy_command.filter(|s| !s.trim().is_empty()),
        identities_only: input.identities_only,
        certificate_file: input.certificate_file.filter(|s| !s.trim().is_empty()),
        compression: input.compression,
        custom_options: input.custom_options.unwrap_or_default(),
        enabled: true,
    };
    config.hosts.push(host.clone());
    validate(&config).map_err(|e| e.to_string())?;
    ssh_forward_config::save(&path, &config).map_err(|e| e.to_string())?;
    Ok(host)
}

#[tauri::command]
fn edit_host(
    original_name: String,
    input: HostInput,
    state: State<'_, AppState>,
) -> Result<Host, String> {
    let path = config_path(&state)?;
    let mut config = load(&path).map_err(|e| e.to_string())?;
    let index = config
        .hosts
        .iter()
        .position(|host| host.name == original_name)
        .ok_or("未找到服务器")?;
    let existing = &config.hosts[index];
    let auth = build_auth(&input, Some(existing))?;
    let host = Host {
        id: existing.id.clone(),
        name: input.name,
        hostname: input.hostname,
        port: input.port,
        username: input.username,
        auth,
        jump_host_id: input.jump_host_id.filter(|s| !s.trim().is_empty()),
        proxy_command: input.proxy_command.filter(|s| !s.trim().is_empty()),
        identities_only: input.identities_only,
        certificate_file: input.certificate_file.filter(|s| !s.trim().is_empty()),
        compression: input.compression,
        custom_options: input.custom_options.unwrap_or_default(),
        enabled: existing.enabled,
    };
    config.hosts[index] = host.clone();
    validate(&config).map_err(|e| e.to_string())?;
    ssh_forward_config::save(&path, &config).map_err(|e| e.to_string())?;
    Ok(host)
}

#[tauri::command]
fn delete_host(name: String, state: State<'_, AppState>) -> Result<(), String> {
    remove_host(&config_path(&state)?, &name).map_err(|error| error.to_string())
}

#[tauri::command]
fn create_tunnel(input: TunnelInput, state: State<'_, AppState>) -> Result<Tunnel, String> {
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
    ssh_forward_core::add_tunnel_full(
        &config_path(&state)?,
        input.name,
        &input.host_name,
        input.kind,
        Endpoint {
            host: input.local_host,
            port: input.local_port,
        },
        remote,
        input.gateway_ports.unwrap_or(false),
        input.custom_options.unwrap_or_default(),
        input.auto_open_browser,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn edit_tunnel(
    original_name: String,
    input: TunnelInput,
    state: State<'_, AppState>,
) -> Result<Tunnel, String> {
    let existing = snapshot(&state)?
        .config
        .tunnels
        .into_iter()
        .find(|tunnel| tunnel.name == original_name)
        .ok_or("未找到 Tunnel")?;
    if state
        .forwards
        .lock()
        .map_err(|_| "Tunnel 运行时锁不可用")?
        .contains_key(&original_name)
    {
        return Err("请先停止 Tunnel 再编辑".into());
    }
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
    update_tunnel(
        &config_path(&state)?,
        &original_name,
        Tunnel {
            id: existing.id,
            name: input.name,
            host_id: existing.host_id,
            kind: input.kind,
            local: Endpoint {
                host: input.local_host,
                port: input.local_port,
            },
            remote,
            gateway_ports: input.gateway_ports.unwrap_or(false),
            custom_options: input.custom_options.unwrap_or_default(),
            auto_start: existing.auto_start,
            auto_reconnect: existing.auto_reconnect,
            auto_open_browser: input.auto_open_browser,
            enabled: existing.enabled,
        },
        &input.host_name,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_tunnel(name: String, state: State<'_, AppState>) -> Result<(), String> {
    stop_tunnel(name.clone(), state.clone())?;
    remove_tunnel(&config_path(&state)?, &name).map_err(|error| error.to_string())
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
        ensure_host_key(&host.hostname, host.port)?;
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
        let forward = start_tunnel_with_password(&config, &name, password.as_deref())
            .map_err(|e| e.to_string())?;
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
            let _ = state.statuses.lock().map(|mut s| {
                s.insert(
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

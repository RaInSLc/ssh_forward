use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use ssh_forward_config::{Auth, AuthType, Config, Endpoint, Host, Tunnel, load, validate};
use ssh_forward_core::{
    add_host_with_auth, add_tunnel, remove_host, remove_tunnel, start_tunnel_with_password,
    update_host, update_tunnel,
};
use ssh_forward_ssh::OpenSshForward;
use tauri::State;
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TunnelInput {
    name: String,
    host_name: String,
    local_host: String,
    local_port: u16,
    remote_host: String,
    remote_port: u16,
}

use serde::Deserialize;

fn config_path(state: &AppState) -> Result<PathBuf, String> {
    state
        .config_path
        .lock()
        .map(|path| path.clone())
        .map_err(|_| "配置路径锁不可用".into())
}

fn default_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("config.json")
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
    let lookup = Command::new("ssh-keygen")
        .args(["-F", &lookup_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("无法检查 SSH Host Key：{error}"))?;
    if lookup.success() {
        return Ok(());
    }

    let scan = Command::new("ssh-keyscan")
        .args([
            "-T",
            "10",
            "-p",
            &port.to_string(),
            "-t",
            "ed25519",
            hostname,
        ])
        .output()
        .map_err(|error| format!("无法获取 SSH Host Key：{error}"))?;
    if !scan.status.success() || scan.stdout.is_empty() {
        return Err("无法获取 SSH Host Key；请检查服务器地址、端口和网络".into());
    }
    let path = known_hosts_path()?;
    let parent = path.parent().ok_or("known_hosts 路径无效")?;
    std::fs::create_dir_all(parent).map_err(|error| format!("无法创建 SSH 配置目录：{error}"))?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("无法保存 SSH Host Key：{error}"))?;
    file.write_all(&scan.stdout)
        .map_err(|error| format!("无法写入 SSH Host Key：{error}"))?;
    if !scan.stdout.ends_with(b"\n") {
        file.write_all(b"\n")
            .map_err(|error| format!("无法完成 SSH Host Key 写入：{error}"))?;
    }
    Ok(())
}

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

#[cfg(test)]
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
                statuses.insert(
                    name.clone(),
                    TunnelStatus {
                        state: "error".into(),
                        message: Some("OpenSSH 进程已退出；请检查认证、Host Key 或网络连接".into()),
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
        config,
        statuses,
    })
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    snapshot(&state)
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
fn create_host(input: HostInput, state: State<'_, AppState>) -> Result<Host, String> {
    let auth = build_auth(&input, None)?;
    add_host_with_auth(
        &config_path(&state)?,
        input.name,
        input.hostname,
        input.port,
        input.username,
        auth,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn edit_host(
    original_name: String,
    input: HostInput,
    state: State<'_, AppState>,
) -> Result<Host, String> {
    let config = snapshot(&state)?.config;
    let existing = config
        .hosts
        .iter()
        .find(|host| host.name == original_name)
        .ok_or("未找到服务器")?;
    let auth = build_auth(&input, Some(existing))?;
    update_host(
        &config_path(&state)?,
        &original_name,
        Host {
            id: existing.id.clone(),
            name: input.name,
            hostname: input.hostname,
            port: input.port,
            username: input.username,
            auth,
            enabled: true,
        },
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_host(name: String, state: State<'_, AppState>) -> Result<(), String> {
    remove_host(&config_path(&state)?, &name).map_err(|error| error.to_string())
}

#[tauri::command]
fn create_tunnel(input: TunnelInput, state: State<'_, AppState>) -> Result<Tunnel, String> {
    add_tunnel(
        &config_path(&state)?,
        input.name,
        &input.host_name,
        Endpoint {
            host: input.local_host,
            port: input.local_port,
        },
        Endpoint {
            host: input.remote_host,
            port: input.remote_port,
        },
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
    update_tunnel(
        &config_path(&state)?,
        &original_name,
        Tunnel {
            id: existing.id,
            name: input.name,
            host_id: existing.host_id,
            kind: existing.kind,
            local: Endpoint {
                host: input.local_host,
                port: input.local_port,
            },
            remote: Endpoint {
                host: input.remote_host,
                port: input.remote_port,
            },
            auto_start: existing.auto_start,
            auto_reconnect: existing.auto_reconnect,
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
    match start_tunnel_with_password(&config, &name, password.as_deref()) {
        Ok(forward) => {
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
                    name,
                    TunnelStatus {
                        state: "running".into(),
                        message: None,
                    },
                );
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            state
                .statuses
                .lock()
                .map_err(|_| "Tunnel 状态锁不可用")?
                .insert(
                    name,
                    TunnelStatus {
                        state: "error".into(),
                        message: Some(message.clone()),
                    },
                );
            Err(message)
        }
    }
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

fn main() {
    tauri::Builder::default()
        .manage(AppState {
            config_path: Mutex::new(default_config_path()),
            forwards: Mutex::new(HashMap::new()),
            statuses: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            set_config_path,
            validate_config,
            create_host,
            edit_host,
            delete_host,
            create_tunnel,
            edit_tunnel,
            delete_tunnel,
            start_tunnel,
            stop_tunnel
        ])
        .run(tauri::generate_context!())
        .expect("error while running SSH Forward desktop application");
}

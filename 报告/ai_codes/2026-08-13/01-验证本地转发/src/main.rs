use std::{io::{Read, Write}, net::TcpStream, path::PathBuf, process::{Command, Stdio}, thread, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ssh_forward_config::{AuthType, load};
use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData}};

fn main() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..");
    let config = load(&root.join("config.json")).map_err(|error| error.to_string())?;
    let tunnel = config.tunnels.iter().find(|item| item.name == "test").ok_or("测试 Tunnel 不存在")?;
    let host = config.hosts.iter().find(|item| item.id == tunnel.host_id).ok_or("测试 Host 不存在")?;
    if host.auth.kind != AuthType::Password { return Err("测试 Host 不是密码认证".into()); }
    let password = unprotect(host.auth.encrypted_password.as_deref().ok_or("缺少 DPAPI 密文")?)?;
    let script = std::env::temp_dir().join(format!("ssh-forward-verify-{}.cmd", std::process::id()));
    std::fs::write(&script, "@echo off\r\npowershell.exe -NoProfile -Command \"[Console]::Out.Write($env:SSH_FORWARD_PASSWORD)\"\r\n").map_err(|error| error.to_string())?;
    let mut child = Command::new("ssh").args(["-N", "-o", "ExitOnForwardFailure=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=10", "-p", &host.port.to_string(), "-L", &format!("{}:{}:{}:{}", tunnel.local.host, tunnel.local.port, tunnel.remote.host, tunnel.remote.port), &format!("{}@{}", host.username, host.hostname)]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).env("SSH_ASKPASS", &script).env("SSH_ASKPASS_REQUIRE", "force").env("DISPLAY", "ssh-forward").env("SSH_FORWARD_PASSWORD", password).spawn().map_err(|error| error.to_string())?;
    thread::sleep(Duration::from_secs(2));
    if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
        let mut error = String::new();
        if let Some(mut stderr) = child.stderr.take() { let _ = stderr.read_to_string(&mut error); }
        let _ = std::fs::remove_file(&script);
        return Err(format!("OpenSSH 提前退出（{status}）：{}", error.lines().next().unwrap_or("未提供错误文本")));
    }
    let mut stream = TcpStream::connect((tunnel.local.host.as_str(), tunnel.local.port)).map_err(|error| format!("本地转发端口无法连接：{error}"))?;
    stream.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n").map_err(|error| error.to_string())?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|error| error.to_string())?;
    let _ = child.kill(); let _ = child.wait(); let _ = std::fs::remove_file(&script);
    println!("HTTP status: {}", response.lines().next().unwrap_or("response received without status line"));
    Ok(())
}

fn unprotect(value: &str) -> Result<String, String> {
    let mut encrypted = STANDARD.decode(value).map_err(|_| "DPAPI 密文格式无效")?;
    let input = CRYPT_INTEGER_BLOB { cbData: encrypted.len().try_into().map_err(|_| "密文长度无效")?, pbData: encrypted.as_mut_ptr() };
    let mut output = CRYPT_INTEGER_BLOB::default();
    if unsafe { CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output) } == 0 { return Err("DPAPI 无法解密当前配置".into()); }
    let password = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe { LocalFree(output.pbData.cast()); }
    String::from_utf8(password).map_err(|_| "DPAPI 输出不是有效文本".into())
}

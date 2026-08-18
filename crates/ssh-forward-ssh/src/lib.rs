use std::{
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
};

use ssh_forward_config::{AuthType, Host, Settings, Tunnel};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SshError {
    #[error("local address {address} is already in use or cannot be bound: {source}")]
    LocalPortUnavailable {
        address: String,
        source: std::io::Error,
    },
    #[error("cannot start OpenSSH: {0}")]
    Start(#[source] std::io::Error),
}

pub fn check_local_port(tunnel: &Tunnel) -> Result<(), SshError> {
    if tunnel.kind == ssh_forward_config::TunnelType::Remote {
        return Ok(());
    }
    let address = format!("{}:{}", tunnel.local.host, tunnel.local.port);
    TcpListener::bind(&address)
        .map(drop)
        .map_err(|source| SshError::LocalPortUnavailable { address, source })
}

pub fn openssh_arguments(
    settings: &Settings,
    host: &Host,
    tunnel: &Tunnel,
    jump_host: Option<&Host>,
) -> Vec<String> {
    let mut arguments = vec![
        "-N".into(),
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={}", settings.connect_timeout_seconds),
        "-o".into(),
        format!(
            "StrictHostKeyChecking={}",
            if settings.strict_host_key_checking {
                "accept-new"
            } else {
                "no"
            }
        ),
        "-o".into(),
        format!(
            "ServerAliveInterval={}",
            settings.server_alive_interval_seconds
        ),
        "-o".into(),
        format!("ServerAliveCountMax={}", settings.server_alive_count_max),
        "-o".into(),
        format!(
            "TCPKeepAlive={}",
            if settings.tcp_keep_alive { "yes" } else { "no" }
        ),
    ];

    if host.compression.unwrap_or(settings.compression) {
        arguments.push("-C".into());
    }

    if tunnel.gateway_ports {
        arguments.push("-g".into());
    }

    if host.identities_only.unwrap_or(false)
        || (host.auth.kind == AuthType::PrivateKey && host.identities_only != Some(false))
    {
        arguments.extend(["-o".into(), "IdentitiesOnly=yes".into()]);
    }

    if let Some(cert) = &host.certificate_file
        && !cert.trim().is_empty()
    {
        arguments.extend(["-o".into(), format!("CertificateFile={cert}")]);
    }

    if let Some(proxy_command) = &host.proxy_command
        && !proxy_command.trim().is_empty()
    {
        arguments.extend(["-o".into(), format!("ProxyCommand={proxy_command}")]);
    }

    if let Some(jump) = jump_host {
        arguments.extend([
            "-J".into(),
            format!("{}@{}:{}", jump.username, jump.hostname, jump.port),
        ]);
    }

    // 自定义 -o 选项（主机级与隧道级）
    for opt in &host.custom_options {
        let trimmed = opt.trim();
        if !trimmed.is_empty() {
            arguments.extend(["-o".into(), trimmed.to_string()]);
        }
    }
    for opt in &tunnel.custom_options {
        let trimmed = opt.trim();
        if !trimmed.is_empty() {
            arguments.extend(["-o".into(), trimmed.to_string()]);
        }
    }

    arguments.extend(["-p".into(), host.port.to_string()]);

    match tunnel.kind {
        ssh_forward_config::TunnelType::Local => {
            if let Some(remote) = &tunnel.remote {
                arguments.extend([
                    "-L".into(),
                    format!(
                        "{}:{}:{}:{}",
                        tunnel.local.host, tunnel.local.port, remote.host, remote.port
                    ),
                ]);
            }
        }
        ssh_forward_config::TunnelType::Dynamic => {
            arguments.extend([
                "-D".into(),
                format!("{}:{}", tunnel.local.host, tunnel.local.port),
            ]);
        }
        ssh_forward_config::TunnelType::Remote => {
            if let Some(remote) = &tunnel.remote {
                arguments.extend([
                    "-R".into(),
                    format!(
                        "{}:{}:{}:{}",
                        remote.host, remote.port, tunnel.local.host, tunnel.local.port
                    ),
                ]);
            }
        }
    }

    if host.auth.kind == AuthType::PrivateKey
        && let Some(private_key) = &host.auth.private_key
    {
        arguments.extend(["-i".into(), private_key.clone()]);
    }
    arguments.push(format!("{}@{}", host.username, host.hostname));
    arguments
}

pub struct OpenSshForward {
    child: Child,
    askpass: Option<PathBuf>,
}

impl OpenSshForward {
    pub fn start(
        settings: &Settings,
        host: &Host,
        tunnel: &Tunnel,
        jump_host: Option<&Host>,
        password: Option<&str>,
    ) -> Result<Self, SshError> {
        check_local_port(tunnel)?;
        let askpass = password.map(|_| create_askpass_script()).transpose()?;
        let mut command = Command::new("ssh");
        command
            .args(openssh_arguments(settings, host, tunnel, jump_host))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        if let Some(password) = password {
            command.env("SSH_ASKPASS", askpass.as_ref().expect("askpass exists"));
            command.env("SSH_ASKPASS_REQUIRE", "force");
            command.env("DISPLAY", "ssh-forward");
            command.env("SSH_FORWARD_PASSWORD", password);
        }
        let child = command.spawn().map_err(SshError::Start)?;
        #[cfg(windows)]
        {
            job::assign_to_job(&child);
        }
        Ok(Self { child, askpass })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn is_running(&mut self) -> Result<bool, SshError> {
        Ok(self.child.try_wait().map_err(SshError::Start)?.is_none())
    }

    pub fn stop(&mut self) -> Result<(), SshError> {
        if self.child.try_wait().map_err(SshError::Start)?.is_none() {
            self.child.kill().map_err(SshError::Start)?;
            self.child.wait().map_err(SshError::Start)?;
        }
        Ok(())
    }
}

impl Drop for OpenSshForward {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(path) = &self.askpass {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn create_askpass_script() -> Result<PathBuf, SshError> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("ssh-forward-{}-{nonce}.cmd", std::process::id()));
    std::fs::write(
        &path,
        "@echo off\r\npowershell.exe -NoProfile -Command \"[Console]::Out.Write($env:SSH_FORWARD_PASSWORD)\"\r\n",
    )
    .map_err(SshError::Start)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh_forward_config::{Auth, Endpoint, TunnelType};

    #[test]
    fn builds_secure_local_forward_command() {
        let settings = Settings::default();
        let host = Host {
            id: "host-1".into(),
            name: "test".into(),
            hostname: "example.test".into(),
            port: 2222,
            username: "alice".into(),
            auth: Auth::default(),
            jump_host_id: None,
            proxy_command: None,
            identities_only: None,
            certificate_file: None,
            compression: None,
            custom_options: vec!["PubkeyAcceptedKeyTypes=+ssh-rsa".into()],
            enabled: true,
        };
        let tunnel = Tunnel {
            id: "tunnel-1".into(),
            name: "web".into(),
            host_id: host.id.clone(),
            kind: TunnelType::Local,
            local: Endpoint::localhost(18080),
            remote: Some(Endpoint::localhost(8080)),
            gateway_ports: true,
            custom_options: vec![],
            auto_start: false,
            auto_reconnect: true,
            auto_open_browser: false,
            enabled: true,
        };
        let args = openssh_arguments(&settings, &host, &tunnel, None);
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-L", "127.0.0.1:18080:127.0.0.1:8080"])
        );
        assert!(args.contains(&"StrictHostKeyChecking=accept-new".into()));
        assert!(args.contains(&"ServerAliveInterval=15".into()));
        assert!(args.contains(&"ServerAliveCountMax=3".into()));
        assert!(args.contains(&"-g".into()));
        assert!(args.contains(&"PubkeyAcceptedKeyTypes=+ssh-rsa".into()));
    }

    #[test]
    fn builds_dynamic_and_jump_command() {
        let settings = Settings::default();
        let jump = Host {
            id: "jump-1".into(),
            name: "bastion".into(),
            hostname: "bastion.test".into(),
            port: 22,
            username: "bastion_user".into(),
            auth: Auth::default(),
            jump_host_id: None,
            proxy_command: None,
            identities_only: None,
            certificate_file: None,
            compression: None,
            custom_options: vec![],
            enabled: true,
        };
        let host = Host {
            id: "host-2".into(),
            name: "internal".into(),
            hostname: "internal.test".into(),
            port: 22,
            username: "bob".into(),
            auth: Auth::default(),
            jump_host_id: Some("jump-1".into()),
            proxy_command: None,
            identities_only: None,
            certificate_file: None,
            compression: Some(true),
            custom_options: vec![],
            enabled: true,
        };
        let tunnel = Tunnel {
            id: "tunnel-dyn".into(),
            name: "socks5".into(),
            host_id: host.id.clone(),
            kind: TunnelType::Dynamic,
            local: Endpoint::localhost(10808),
            remote: None,
            gateway_ports: false,
            custom_options: vec![],
            auto_start: false,
            auto_reconnect: true,
            auto_open_browser: false,
            enabled: true,
        };
        let args = openssh_arguments(&settings, &host, &tunnel, Some(&jump));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-J", "bastion_user@bastion.test:22"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-D", "127.0.0.1:10808"])
        );
        assert!(args.contains(&"-C".into()));
    }
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    struct SafeJob(HANDLE);
    unsafe impl Send for SafeJob {}
    unsafe impl Sync for SafeJob {}

    impl Drop for SafeJob {
        fn drop(&mut self) {
            unsafe {
                if self.0 != 0 as HANDLE {
                    CloseHandle(self.0);
                }
            }
        }
    }

    static GLOBAL_JOB: OnceLock<SafeJob> = OnceLock::new();

    pub fn assign_to_job(child: &std::process::Child) {
        let job = GLOBAL_JOB.get_or_init(|| unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle != 0 as HANDLE {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
            }
            SafeJob(handle)
        });

        if job.0 != 0 as HANDLE {
            unsafe {
                AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE);
            }
        }
    }
}

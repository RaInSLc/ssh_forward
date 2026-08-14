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
    let address = format!("{}:{}", tunnel.local.host, tunnel.local.port);
    TcpListener::bind(&address)
        .map(drop)
        .map_err(|source| SshError::LocalPortUnavailable { address, source })
}

pub fn openssh_arguments(settings: &Settings, host: &Host, tunnel: &Tunnel) -> Vec<String> {
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
                "yes"
            } else {
                "no"
            }
        ),
        "-p".into(),
        host.port.to_string(),
        "-L".into(),
        format!(
            "{}:{}:{}:{}",
            tunnel.local.host, tunnel.local.port, tunnel.remote.host, tunnel.remote.port
        ),
    ];
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
        password: Option<&str>,
    ) -> Result<Self, SshError> {
        check_local_port(tunnel)?;
        let askpass = password.map(|_| create_askpass_script()).transpose()?;
        let mut command = Command::new("ssh");
        command
            .args(openssh_arguments(settings, host, tunnel))
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
            enabled: true,
        };
        let tunnel = Tunnel {
            id: "tunnel-1".into(),
            name: "web".into(),
            host_id: host.id.clone(),
            kind: TunnelType::Local,
            local: Endpoint::localhost(18080),
            remote: Endpoint::localhost(8080),
            auto_start: false,
            auto_reconnect: true,
            auto_open_browser: false,
            enabled: true,
        };
        let args = openssh_arguments(&settings, &host, &tunnel);
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-L", "127.0.0.1:18080:127.0.0.1:8080"])
        );
        assert!(args.contains(&"StrictHostKeyChecking=yes".into()));
    }
}

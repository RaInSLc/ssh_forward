use std::{collections::HashSet, io, path::PathBuf};

use thiserror::Error;

use crate::{AuthType, CONFIG_VERSION, Config, TunnelType};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read configuration {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot parse configuration {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("cannot create configuration directory {path}: {source}")]
    CreateDirectory { path: PathBuf, source: io::Error },
    #[error("cannot write configuration {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot replace configuration {temporary_path} with {path}: {source}")]
    Replace {
        temporary_path: PathBuf,
        path: PathBuf,
        source: io::Error,
    },
    #[error("configuration validation failed: {0}")]
    Validation(String),
}

pub fn validate(config: &Config) -> Result<(), ConfigError> {
    if config.version != CONFIG_VERSION {
        return Err(ConfigError::Validation(format!(
            "unsupported configuration version {}; expected {CONFIG_VERSION}",
            config.version
        )));
    }
    if config.settings.connect_timeout_seconds == 0 {
        return Err(ConfigError::Validation(
            "connect_timeout_seconds must be at least 1".into(),
        ));
    }

    let mut ids = HashSet::new();
    let mut host_ids = HashSet::new();
    for host in &config.hosts {
        require_id(&mut ids, &host.id, "host")?;
        host_ids.insert(host.id.as_str());
        require_nonempty(&host.name, "host name")?;
        require_nonempty(&host.hostname, "host hostname")?;
        require_nonempty(&host.username, "host username")?;
        require_port(host.port, "host port")?;
        if let Some(jump_id) = &host.jump_host_id
            && jump_id == &host.id
        {
            return Err(ConfigError::Validation(format!(
                "host '{}' cannot reference itself as jump_host_id",
                host.name
            )));
        }
        if host.auth.kind == AuthType::PrivateKey {
            match &host.auth.private_key {
                Some(path) if !path.trim().is_empty() => {}
                _ => {
                    return Err(ConfigError::Validation(format!(
                        "host '{}' uses private_key authentication but has no private_key path",
                        host.name
                    )));
                }
            }
        }
        if host.auth.kind == AuthType::Password {
            let has_encrypted_password = host
                .auth
                .encrypted_password
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            let has_legacy_credential = host
                .auth
                .credential_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
            if !has_encrypted_password && !has_legacy_credential {
                return Err(ConfigError::Validation(format!(
                    "host '{}' uses password authentication but has no encrypted_password",
                    host.name
                )));
            }
        }
    }
    for host in &config.hosts {
        if let Some(jump_id) = &host.jump_host_id
            && !host_ids.contains(jump_id.as_str())
        {
            return Err(ConfigError::Validation(format!(
                "host '{}' references unknown jump_host_id '{jump_id}'",
                host.name
            )));
        }
    }
    for tunnel in &config.tunnels {
        require_id(&mut ids, &tunnel.id, "tunnel")?;
        require_nonempty(&tunnel.name, "tunnel name")?;
        if !host_ids.contains(tunnel.host_id.as_str()) {
            return Err(ConfigError::Validation(format!(
                "tunnel '{}' references unknown host_id '{}'",
                tunnel.name, tunnel.host_id
            )));
        }
        require_nonempty(&tunnel.local.host, "local host")?;
        require_port(tunnel.local.port, "local port")?;

        match tunnel.kind {
            TunnelType::Local | TunnelType::Remote => {
                let remote = tunnel.remote.as_ref().ok_or_else(|| {
                    ConfigError::Validation(format!(
                        "tunnel '{}' is {:?} but missing remote endpoint",
                        tunnel.name, tunnel.kind
                    ))
                })?;
                require_nonempty(&remote.host, "remote host")?;
                require_port(remote.port, "remote port")?;
            }
            TunnelType::Dynamic => {
                // Dynamic SOCKS5 只需绑定本地端口，远端无需单独配置
            }
        }

        if tunnel.local.host != "127.0.0.1"
            && tunnel.local.host != "::1"
            && tunnel.local.host != "localhost"
            && tunnel.local.host != "0.0.0.0"
        {
            return Err(ConfigError::Validation(format!(
                "tunnel '{}' binds to unsupported address '{}'",
                tunnel.name, tunnel.local.host
            )));
        }
    }
    Ok(())
}

fn require_id(ids: &mut HashSet<String>, id: &str, kind: &str) -> Result<(), ConfigError> {
    require_nonempty(id, &format!("{kind} id"))?;
    if !ids.insert(id.into()) {
        return Err(ConfigError::Validation(format!(
            "duplicate {kind} id '{id}'"
        )));
    }
    Ok(())
}

fn require_nonempty(value: &str, field: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        Err(ConfigError::Validation(format!("{field} cannot be empty")))
    } else {
        Ok(())
    }
}

fn require_port(port: u16, field: &str) -> Result<(), ConfigError> {
    if port == 0 {
        Err(ConfigError::Validation(format!(
            "{field} must be in range 1..=65535"
        )))
    } else {
        Ok(())
    }
}

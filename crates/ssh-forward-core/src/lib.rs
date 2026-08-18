use std::path::Path;

use ssh_forward_config::{
    Auth, AuthType, Config, ConfigError, Endpoint, Host, Tunnel, TunnelType, load, save,
};
use ssh_forward_ssh::{OpenSshForward, SshError};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Ssh(#[from] SshError),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    AlreadyExists(String),
}

pub fn add_host(
    path: &Path,
    name: String,
    hostname: String,
    port: u16,
    username: String,
    private_key: Option<String>,
) -> Result<Host, CoreError> {
    let auth = match private_key {
        Some(private_key) => Auth {
            kind: AuthType::PrivateKey,
            private_key: Some(private_key),
            credential_id: None,
            encrypted_password: None,
        },
        None => Auth::default(),
    };
    add_host_with_auth(path, name, hostname, port, username, auth)
}

pub fn add_host_with_auth(
    path: &Path,
    name: String,
    hostname: String,
    port: u16,
    username: String,
    auth: Auth,
) -> Result<Host, CoreError> {
    let mut config = load(path)?;
    if config.hosts.iter().any(|host| host.name == name) {
        return Err(CoreError::AlreadyExists(format!(
            "host named '{name}' already exists"
        )));
    }
    let host = Host {
        id: Uuid::new_v4().to_string(),
        name,
        hostname,
        port,
        username,
        auth,
        jump_host_id: None,
        proxy_command: None,
        identities_only: None,
        certificate_file: None,
        compression: None,
        custom_options: Vec::new(),
        enabled: true,
    };
    config.hosts.push(host.clone());
    save(path, &config)?;
    Ok(host)
}

pub fn remove_host(path: &Path, name: &str) -> Result<(), CoreError> {
    let mut config = load(path)?;
    let position = config
        .hosts
        .iter()
        .position(|host| host.name == name)
        .ok_or_else(|| CoreError::NotFound(format!("host '{name}' was not found")))?;
    let host_id = config.hosts[position].id.clone();
    if config
        .tunnels
        .iter()
        .any(|tunnel| tunnel.host_id == host_id)
    {
        return Err(CoreError::AlreadyExists(format!(
            "host '{name}' still has tunnels; remove them first"
        )));
    }
    config.hosts.remove(position);
    save(path, &config)?;
    Ok(())
}

pub fn update_host(path: &Path, original_name: &str, host: Host) -> Result<Host, CoreError> {
    let mut config = load(path)?;
    let index = config
        .hosts
        .iter()
        .position(|item| item.name == original_name)
        .ok_or_else(|| CoreError::NotFound(format!("host '{original_name}' was not found")))?;
    if host.name != original_name && config.hosts.iter().any(|item| item.name == host.name) {
        return Err(CoreError::AlreadyExists(format!(
            "host named '{}' already exists",
            host.name
        )));
    }
    let mut host = host;
    host.id = config.hosts[index].id.clone();
    config.hosts[index] = host.clone();
    save(path, &config)?;
    Ok(host)
}

pub fn add_tunnel(
    path: &Path,
    name: String,
    host_name: &str,
    local: Endpoint,
    remote: Option<Endpoint>,
    auto_open_browser: bool,
) -> Result<Tunnel, CoreError> {
    add_tunnel_full(
        path,
        name,
        host_name,
        TunnelType::Local,
        local,
        remote,
        false,
        Vec::new(),
        auto_open_browser,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn add_tunnel_full(
    path: &Path,
    name: String,
    host_name: &str,
    kind: TunnelType,
    local: Endpoint,
    remote: Option<Endpoint>,
    gateway_ports: bool,
    custom_options: Vec<String>,
    auto_open_browser: bool,
) -> Result<Tunnel, CoreError> {
    let mut config = load(path)?;
    if config.tunnels.iter().any(|tunnel| tunnel.name == name) {
        return Err(CoreError::AlreadyExists(format!(
            "tunnel named '{name}' already exists"
        )));
    }
    let host_id = config
        .hosts
        .iter()
        .find(|host| host.name == host_name)
        .ok_or_else(|| CoreError::NotFound(format!("host '{host_name}' was not found")))?
        .id
        .clone();
    let tunnel = Tunnel {
        id: Uuid::new_v4().to_string(),
        name,
        host_id,
        kind,
        local,
        remote,
        gateway_ports,
        custom_options,
        auto_start: false,
        auto_reconnect: true,
        auto_open_browser,
        enabled: true,
    };
    config.tunnels.push(tunnel.clone());
    save(path, &config)?;
    Ok(tunnel)
}

pub fn remove_tunnel(path: &Path, name: &str) -> Result<(), CoreError> {
    let mut config = load(path)?;
    let position = config
        .tunnels
        .iter()
        .position(|tunnel| tunnel.name == name)
        .ok_or_else(|| CoreError::NotFound(format!("tunnel '{name}' was not found")))?;
    config.tunnels.remove(position);
    save(path, &config)?;
    Ok(())
}

pub fn update_tunnel(
    path: &Path,
    original_name: &str,
    tunnel: Tunnel,
    host_name: &str,
) -> Result<Tunnel, CoreError> {
    let mut config = load(path)?;
    let index = config
        .tunnels
        .iter()
        .position(|item| item.name == original_name)
        .ok_or_else(|| CoreError::NotFound(format!("tunnel '{original_name}' was not found")))?;
    if tunnel.name != original_name && config.tunnels.iter().any(|item| item.name == tunnel.name) {
        return Err(CoreError::AlreadyExists(format!(
            "tunnel named '{}' already exists",
            tunnel.name
        )));
    }
    let host_id = config
        .hosts
        .iter()
        .find(|host| host.name == host_name)
        .ok_or_else(|| CoreError::NotFound(format!("host '{host_name}' was not found")))?
        .id
        .clone();
    let mut tunnel = tunnel;
    tunnel.id = config.tunnels[index].id.clone();
    tunnel.host_id = host_id;
    config.tunnels[index] = tunnel.clone();
    save(path, &config)?;
    Ok(tunnel)
}

pub fn start_tunnel(config: &Config, name: &str) -> Result<OpenSshForward, CoreError> {
    start_tunnel_with_password(config, name, None)
}

pub fn start_tunnel_with_password(
    config: &Config,
    name: &str,
    password: Option<&str>,
) -> Result<OpenSshForward, CoreError> {
    let tunnel = config
        .tunnels
        .iter()
        .find(|tunnel| tunnel.name == name)
        .ok_or_else(|| CoreError::NotFound(format!("tunnel '{name}' was not found")))?;
    if !tunnel.enabled {
        return Err(CoreError::NotFound(format!("tunnel '{name}' is disabled")));
    }
    let host = config
        .hosts
        .iter()
        .find(|host| host.id == tunnel.host_id)
        .ok_or_else(|| CoreError::NotFound(format!("host for tunnel '{name}' was not found")))?;
    if !host.enabled {
        return Err(CoreError::NotFound(format!(
            "host '{}' is disabled",
            host.name
        )));
    }
    if host.auth.kind == AuthType::Password && password.is_none() {
        return Err(CoreError::NotFound(format!(
            "password credential for host '{}' was not found",
            host.name
        )));
    }
    let jump_host = host
        .jump_host_id
        .as_deref()
        .and_then(|jump_id| config.hosts.iter().find(|h| h.id == jump_id));
    Ok(OpenSshForward::start(
        &config.settings,
        host,
        tunnel,
        jump_host,
        password,
    )?)
}

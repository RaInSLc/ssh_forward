mod runtime;

use std::path::Path;

use ssh_forward_config::{
    Auth, AuthType, Config, ConfigError, ConfigLock, Endpoint, Host, Tunnel, TunnelType, load, save,
};
use ssh_forward_ssh::{ForwardSpec, OpenSshForward, SshError};
use thiserror::Error;
use uuid::Uuid;

pub use runtime::{
    RuntimePaths, app_data_dir, known_hosts_path, sanitize_file_stem, tunnel_log_path,
};

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
    /// 操作与当前状态冲突（例如服务器仍有关联 Tunnel）。
    #[error("{0}")]
    Conflict(String),
    /// 目标被显式禁用。
    #[error("{0}")]
    Disabled(String),
    /// 缺少完成操作所需的凭据。
    #[error("{0}")]
    CredentialMissing(String),
    /// 输入本身不合法。
    #[error("{0}")]
    Invalid(String),
}

/// 新建或编辑服务器时的认证输入。
///
/// `Password` 的密文由调用方（掌握平台加密能力的壳层）生成；传 `None`
/// 表示沿用配置中已有的密文，从而避免编辑其它字段时要求用户重新输入密码。
#[derive(Debug, Clone)]
pub enum AuthInput {
    SshAgent,
    PrivateKey { path: String },
    Password { encrypted_password: Option<String> },
}

/// 服务器的完整写入意图。
///
/// 所有字段都必须显式给出，避免"表单没有渲染这个字段"被误解成"清空这个字段"。
#[derive(Debug, Clone)]
pub struct HostDraft {
    pub name: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthInput,
    pub jump_host_id: Option<String>,
    pub proxy_command: Option<String>,
    pub identities_only: Option<bool>,
    pub certificate_file: Option<String>,
    pub compression: Option<bool>,
    pub custom_options: Vec<String>,
    pub enabled: bool,
}

/// 转发的完整写入意图。
#[derive(Debug, Clone)]
pub struct TunnelDraft {
    pub name: String,
    pub host_name: String,
    pub kind: TunnelType,
    pub local: Endpoint,
    pub remote: Option<Endpoint>,
    pub gateway_ports: bool,
    pub custom_options: Vec<String>,
    pub auto_open_browser: bool,
}

/// 启动转发的运行期选项，由壳层提供平台相关路径与凭据。
#[derive(Debug, Clone, Default)]
pub struct StartOptions<'a> {
    pub password: Option<&'a str>,
    /// 应用私有 known_hosts 文件，避免污染用户主目录下的同名文件。
    pub known_hosts: Option<&'a Path>,
    /// OpenSSH stderr 的落盘路径。
    pub log_path: Option<&'a Path>,
}

fn clean(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn resolve_auth(input: &AuthInput, existing: Option<&Host>) -> Result<Auth, CoreError> {
    match input {
        AuthInput::SshAgent => Ok(Auth::default()),
        AuthInput::PrivateKey { path } => {
            if path.trim().is_empty() {
                return Err(CoreError::Invalid("私钥认证需要提供私钥路径".into()));
            }
            Ok(Auth {
                kind: AuthType::PrivateKey,
                private_key: Some(path.clone()),
                credential_id: None,
                encrypted_password: None,
            })
        }
        AuthInput::Password { encrypted_password } => {
            let encrypted = match encrypted_password
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                Some(value) => value.to_owned(),
                None => existing
                    .filter(|host| host.auth.kind == AuthType::Password)
                    .and_then(|host| host.auth.encrypted_password.clone())
                    .ok_or_else(|| CoreError::CredentialMissing("密码认证需要输入密码".into()))?,
            };
            Ok(Auth {
                kind: AuthType::Password,
                private_key: None,
                credential_id: None,
                encrypted_password: Some(encrypted),
            })
        }
    }
}

/// 新建（`original_name` 为 `None`）或编辑服务器。
///
/// 整个过程在同一把跨进程文件锁内完成，避免并发写入互相覆盖。
pub fn upsert_host(
    path: &Path,
    original_name: Option<&str>,
    draft: HostDraft,
) -> Result<Host, CoreError> {
    let _guard = ConfigLock::acquire(path)?;
    let mut config = load(path)?;

    let index = match original_name {
        Some(name) => Some(
            config
                .hosts
                .iter()
                .position(|host| host.name == name)
                .ok_or_else(|| CoreError::NotFound(format!("host '{name}' was not found")))?,
        ),
        None => None,
    };

    let duplicated = config
        .hosts
        .iter()
        .enumerate()
        .any(|(position, host)| host.name == draft.name && Some(position) != index);
    if duplicated {
        return Err(CoreError::AlreadyExists(format!(
            "host named '{}' already exists",
            draft.name
        )));
    }

    let (id, enabled) = match index {
        Some(position) => (
            config.hosts[position].id.clone(),
            config.hosts[position].enabled,
        ),
        None => (Uuid::new_v4().to_string(), draft.enabled),
    };
    let auth = {
        let existing = index.map(|position| &config.hosts[position]);
        resolve_auth(&draft.auth, existing)?
    };

    let host = Host {
        id,
        name: draft.name,
        hostname: draft.hostname,
        port: draft.port,
        username: draft.username,
        auth,
        jump_host_id: clean(draft.jump_host_id),
        proxy_command: clean(draft.proxy_command),
        identities_only: draft.identities_only,
        certificate_file: clean(draft.certificate_file),
        compression: draft.compression,
        custom_options: draft.custom_options,
        enabled,
    };

    match index {
        Some(position) => config.hosts[position] = host.clone(),
        None => config.hosts.push(host.clone()),
    }
    save(path, &config)?;
    Ok(host)
}

pub fn remove_host(path: &Path, name: &str) -> Result<(), CoreError> {
    let _guard = ConfigLock::acquire(path)?;
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
        return Err(CoreError::Conflict(format!(
            "host '{name}' still has tunnels; remove them first"
        )));
    }
    config.hosts.remove(position);
    save(path, &config)?;
    Ok(())
}

/// 新建（`original_name` 为 `None`）或编辑转发。
pub fn upsert_tunnel(
    path: &Path,
    original_name: Option<&str>,
    draft: TunnelDraft,
) -> Result<Tunnel, CoreError> {
    let _guard = ConfigLock::acquire(path)?;
    let mut config = load(path)?;

    let index = match original_name {
        Some(name) => Some(
            config
                .tunnels
                .iter()
                .position(|tunnel| tunnel.name == name)
                .ok_or_else(|| CoreError::NotFound(format!("tunnel '{name}' was not found")))?,
        ),
        None => None,
    };

    let duplicated = config
        .tunnels
        .iter()
        .enumerate()
        .any(|(position, tunnel)| tunnel.name == draft.name && Some(position) != index);
    if duplicated {
        return Err(CoreError::AlreadyExists(format!(
            "tunnel named '{}' already exists",
            draft.name
        )));
    }

    let host_id = config
        .hosts
        .iter()
        .find(|host| host.name == draft.host_name)
        .ok_or_else(|| CoreError::NotFound(format!("host '{}' was not found", draft.host_name)))?
        .id
        .clone();

    let (id, auto_start, auto_reconnect, enabled) = match index {
        Some(position) => {
            let existing = &config.tunnels[position];
            (
                existing.id.clone(),
                existing.auto_start,
                existing.auto_reconnect,
                existing.enabled,
            )
        }
        None => (Uuid::new_v4().to_string(), false, true, true),
    };

    let tunnel = Tunnel {
        id,
        name: draft.name,
        host_id,
        kind: draft.kind,
        local: draft.local,
        remote: draft.remote,
        gateway_ports: draft.gateway_ports,
        custom_options: draft.custom_options,
        auto_start,
        auto_reconnect,
        auto_open_browser: draft.auto_open_browser,
        enabled,
    };

    match index {
        Some(position) => config.tunnels[position] = tunnel.clone(),
        None => config.tunnels.push(tunnel.clone()),
    }
    save(path, &config)?;
    Ok(tunnel)
}

pub fn remove_tunnel(path: &Path, name: &str) -> Result<(), CoreError> {
    let _guard = ConfigLock::acquire(path)?;
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

/// 按名称启动一个转发。
pub fn start_tunnel(
    config: &Config,
    name: &str,
    options: StartOptions<'_>,
) -> Result<OpenSshForward, CoreError> {
    let tunnel = config
        .tunnels
        .iter()
        .find(|tunnel| tunnel.name == name)
        .ok_or_else(|| CoreError::NotFound(format!("tunnel '{name}' was not found")))?;
    if !tunnel.enabled {
        return Err(CoreError::Disabled(format!("tunnel '{name}' is disabled")));
    }
    let host = config
        .hosts
        .iter()
        .find(|host| host.id == tunnel.host_id)
        .ok_or_else(|| CoreError::NotFound(format!("host for tunnel '{name}' was not found")))?;
    if !host.enabled {
        return Err(CoreError::Disabled(format!(
            "host '{}' is disabled",
            host.name
        )));
    }
    if host.auth.kind == AuthType::Password && options.password.is_none() {
        return Err(CoreError::CredentialMissing(format!(
            "password credential for host '{}' was not found",
            host.name
        )));
    }
    let jump_host = host
        .jump_host_id
        .as_deref()
        .and_then(|jump_id| config.hosts.iter().find(|host| host.id == jump_id));

    Ok(OpenSshForward::start(&ForwardSpec {
        settings: &config.settings,
        host,
        tunnel,
        jump_host,
        password: options.password,
        known_hosts: options.known_hosts,
        log_path: options.log_path,
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh_forward_config::{AuthType, HostKeyPolicy};

    fn scratch_path(name: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time is after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ssh-forward-core-test-{}-{nonce}-{name}",
            std::process::id()
        ))
    }

    fn host_draft(name: &str) -> HostDraft {
        HostDraft {
            name: name.into(),
            hostname: "example.test".into(),
            port: 22,
            username: "alice".into(),
            auth: AuthInput::SshAgent,
            jump_host_id: None,
            proxy_command: None,
            identities_only: None,
            certificate_file: None,
            compression: None,
            custom_options: Vec::new(),
            enabled: true,
        }
    }

    fn tunnel_draft(name: &str, host_name: &str) -> TunnelDraft {
        TunnelDraft {
            name: name.into(),
            host_name: host_name.into(),
            kind: TunnelType::Local,
            local: Endpoint::localhost(18080),
            remote: Some(Endpoint::localhost(80)),
            gateway_ports: false,
            custom_options: Vec::new(),
            auto_open_browser: false,
        }
    }

    #[test]
    fn creates_and_updates_a_host_without_touching_its_id() {
        let path = scratch_path("host.json");

        let created = upsert_host(&path, None, host_draft("alpha")).unwrap();
        assert_eq!(created.name, "alpha");

        // 跳板机必须是**另一个**主机：把主机自己当作 jump_host_id 会被校验拒绝
        // （"host 'alpha' cannot reference itself as jump_host_id"）。
        let jump = upsert_host(&path, None, host_draft("beta")).unwrap();

        let mut draft = host_draft("alpha");
        draft.port = 2200;
        draft.jump_host_id = Some(jump.id.clone());
        let updated = upsert_host(&path, Some("alpha"), draft).unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.port, 2200);
        assert_eq!(updated.jump_host_id.as_deref(), Some(jump.id.as_str()));

        let config = load(&path).unwrap();
        assert_eq!(config.hosts.len(), 2);
        let alpha = config
            .hosts
            .iter()
            .find(|host| host.name == "alpha")
            .expect("alpha should still exist after the update");
        assert_eq!(alpha.port, 2200);
        assert_eq!(alpha.id, created.id);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_a_duplicate_host_name_on_create() {
        let path = scratch_path("duplicate.json");
        upsert_host(&path, None, host_draft("alpha")).unwrap();
        let error = upsert_host(&path, None, host_draft("alpha")).unwrap_err();
        assert!(matches!(error, CoreError::AlreadyExists(_)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn keeps_the_stored_password_when_the_draft_omits_it() {
        let path = scratch_path("password.json");
        let mut draft = host_draft("alpha");
        draft.auth = AuthInput::Password {
            encrypted_password: Some("cipher-text".into()),
        };
        upsert_host(&path, None, draft).unwrap();

        let mut edit = host_draft("alpha");
        edit.port = 2222;
        edit.auth = AuthInput::Password {
            encrypted_password: None,
        };
        let updated = upsert_host(&path, Some("alpha"), edit).unwrap();

        assert_eq!(updated.auth.kind, AuthType::Password);
        assert_eq!(
            updated.auth.encrypted_password.as_deref(),
            Some("cipher-text")
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn refuses_to_delete_a_host_that_still_has_tunnels() {
        let path = scratch_path("conflict.json");
        upsert_host(&path, None, host_draft("alpha")).unwrap();
        upsert_tunnel(&path, None, tunnel_draft("web", "alpha")).unwrap();

        let error = remove_host(&path, "alpha").unwrap_err();
        assert!(matches!(error, CoreError::Conflict(_)));

        remove_tunnel(&path, "web").unwrap();
        remove_host(&path, "alpha").unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn preserves_runtime_flags_when_editing_a_tunnel() {
        let path = scratch_path("tunnel.json");
        upsert_host(&path, None, host_draft("alpha")).unwrap();
        let created = upsert_tunnel(&path, None, tunnel_draft("web", "alpha")).unwrap();
        assert!(!created.id.is_empty());

        let mut draft = tunnel_draft("web", "alpha");
        draft.local = Endpoint::localhost(19090);
        let updated = upsert_tunnel(&path, Some("web"), draft).unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.auto_reconnect, created.auto_reconnect);
        assert_eq!(updated.local.port, 19090);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reports_disabled_targets_distinctly() {
        let path = scratch_path("disabled.json");
        let mut draft = host_draft("alpha");
        draft.enabled = false;
        let host = upsert_host(&path, None, draft).unwrap();
        assert!(!host.enabled);

        let mut tunnel = tunnel_draft("web", "alpha");
        tunnel.kind = TunnelType::Dynamic;
        tunnel.remote = None;
        let created = upsert_tunnel(&path, None, tunnel).unwrap();

        let config = load(&path).unwrap();
        let error = match start_tunnel(&config, &created.name, StartOptions::default()) {
            Ok(_) => panic!("disabled host should be rejected before starting OpenSSH"),
            Err(error) => error,
        };
        assert!(matches!(error, CoreError::Disabled(_)));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn host_key_policy_default_is_accept_new() {
        assert_eq!(
            Config::default().settings.host_key_policy,
            HostKeyPolicy::AcceptNew
        );
    }
}

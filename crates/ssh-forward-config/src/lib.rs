mod model;
mod validation;

use std::{fs, io::Write, path::Path};

pub use model::*;
pub use validation::{ConfigError, validate};

pub const CONFIG_VERSION: u32 = 1;

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    if !path.exists() {
        return Ok(Config::default());
    }

    let content = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let config = serde_json::from_str(&content).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    validate(&config)?;
    Ok(config)
}

pub fn save(path: &Path, config: &Config) -> Result<(), ConfigError> {
    validate(config)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let temporary_path = path.with_extension("json.tmp");
    let serialized = serde_json::to_vec_pretty(config).expect("config serialization cannot fail");
    let mut temporary = fs::File::create(&temporary_path).map_err(|source| ConfigError::Write {
        path: temporary_path.clone(),
        source,
    })?;
    temporary
        .write_all(&serialized)
        .map_err(|source| ConfigError::Write {
            path: temporary_path.clone(),
            source,
        })?;
    temporary
        .write_all(b"\n")
        .map_err(|source| ConfigError::Write {
            path: temporary_path.clone(),
            source,
        })?;
    temporary.sync_all().map_err(|source| ConfigError::Write {
        path: temporary_path.clone(),
        source,
    })?;
    drop(temporary);
    fs::rename(&temporary_path, path).map_err(|source| ConfigError::Replace {
        temporary_path,
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        validate(&Config::default()).unwrap();
    }

    #[test]
    fn tunnel_requires_an_existing_host() {
        let mut config = Config::default();
        config.tunnels.push(Tunnel {
            id: "tunnel-1".into(),
            name: "database".into(),
            host_id: "missing".into(),
            kind: TunnelType::Local,
            local: Endpoint::localhost(13306),
            remote: Endpoint::localhost(3306),
            auto_start: false,
            auto_reconnect: true,
            auto_open_browser: false,
            enabled: true,
        });
        assert!(validate(&config).is_err());
    }

    #[test]
    fn password_authentication_requires_an_encrypted_value() {
        let mut config = Config::default();
        config.hosts.push(Host {
            id: "host-1".into(),
            name: "password-host".into(),
            hostname: "example.test".into(),
            port: 22,
            username: "alice".into(),
            auth: Auth {
                kind: AuthType::Password,
                private_key: None,
                credential_id: None,
                encrypted_password: Some("encrypted-value".into()),
            },
            enabled: true,
        });
        validate(&config).unwrap();
        let encoded = serde_json::to_string(&config).unwrap();
        assert!(encoded.contains("encrypted_password"));
        assert!(!encoded.contains("secret-value"));
    }

    #[test]
    fn legacy_password_credential_remains_loadable_for_migration() {
        let mut config = Config::default();
        config.hosts.push(Host {
            id: "host-legacy".into(),
            name: "legacy-password-host".into(),
            hostname: "example.test".into(),
            port: 22,
            username: "alice".into(),
            auth: Auth {
                kind: AuthType::Password,
                private_key: None,
                credential_id: Some("legacy-credential".into()),
                encrypted_password: None,
            },
            enabled: true,
        });
        validate(&config).unwrap();
    }
}

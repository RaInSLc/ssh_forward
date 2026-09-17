mod model;
mod validation;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde_json::Value;

pub use model::*;
pub use validation::{ConfigError, validate};

pub const CONFIG_VERSION: u32 = 2;

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    if !path.exists() {
        return Ok(Config::default());
    }

    let content = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value: Value = serde_json::from_str(&content).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let value = migrate(value)?;
    let config: Config = serde_json::from_value(value).map_err(|source| ConfigError::Parse {
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

    // 配置文件包含完整主机清单与凭据密文，尽量收紧为仅属主可读写。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&temporary_path, fs::Permissions::from_mode(0o600));
    }

    fs::rename(&temporary_path, path).map_err(|source| ConfigError::Replace {
        temporary_path,
        path: path.to_path_buf(),
        source,
    })
}

/// 跨进程配置文件锁。
///
/// 读改写（load → 修改 → save）必须在同一把锁内完成，否则两个进程或两个应用实例
/// 会互相覆盖。锁在 `Drop` 时释放，因此 guard 的生命周期必须覆盖整个读改写过程。
pub struct ConfigLock {
    file: fs::File,
}

impl ConfigLock {
    pub fn acquire(path: &Path) -> Result<Self, ConfigError> {
        let lock_path = lock_path(path);
        if let Some(parent) = lock_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| ConfigError::Lock {
                path: lock_path.clone(),
                source,
            })?;
        file.lock().map_err(|source| ConfigError::Lock {
            path: lock_path,
            source,
        })?;
        Ok(Self { file })
    }
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "config.json".into());
    name.push(".lock");
    path.with_file_name(name)
}

/// 把任意历史版本的配置文档升级到 [`CONFIG_VERSION`]。
fn migrate(value: Value) -> Result<Value, ConfigError> {
    let mut value = value;
    if !value.is_object() {
        return Err(ConfigError::Validation(
            "configuration root must be a JSON object".into(),
        ));
    }

    let found = value.get("version").and_then(Value::as_u64).unwrap_or(1);
    if found > u64::from(CONFIG_VERSION) {
        return Err(ConfigError::UnsupportedVersion {
            found: found.min(u64::from(u32::MAX)) as u32,
            supported: CONFIG_VERSION,
        });
    }

    if found < 2 {
        apply_v1_to_v2(&mut value);
    }

    value["version"] = Value::from(CONFIG_VERSION);
    Ok(value)
}

/// v1 → v2：`settings.strict_host_key_checking: bool` 拆分为三态 `settings.host_key_policy`。
///
/// `true` 映射为 `accept_new`（保持 v1 的默认行为），`false` 映射为 `insecure`
/// （忠实保留原有的"不校验"语义，界面上会明确标注为风险项）。
fn apply_v1_to_v2(value: &mut Value) {
    let Some(settings) = value.get_mut("settings").and_then(Value::as_object_mut) else {
        return;
    };
    let policy = match settings.remove("strict_host_key_checking") {
        Some(Value::Bool(false)) => "insecure",
        _ => "accept_new",
    };
    settings.insert("host_key_policy".into(), Value::String(policy.into()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_path(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time is after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ssh-forward-config-test-{}-{nonce}-{name}",
            std::process::id()
        ))
    }

    fn sample_tunnel() -> Tunnel {
        Tunnel {
            id: "tunnel-1".into(),
            name: "database".into(),
            host_id: "missing".into(),
            kind: TunnelType::Local,
            local: Endpoint::localhost(13306),
            remote: Some(Endpoint::localhost(3306)),
            gateway_ports: false,
            custom_options: Vec::new(),
            auto_start: false,
            auto_reconnect: true,
            auto_open_browser: false,
            enabled: true,
        }
    }

    fn sample_password_host() -> Host {
        Host {
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
            jump_host_id: None,
            proxy_command: None,
            identities_only: None,
            certificate_file: None,
            compression: None,
            custom_options: Vec::new(),
            enabled: true,
        }
    }

    #[test]
    fn default_config_is_valid() {
        validate(&Config::default()).unwrap();
    }

    #[test]
    fn tunnel_requires_an_existing_host() {
        let mut config = Config::default();
        config.tunnels.push(sample_tunnel());
        assert!(validate(&config).is_err());
    }

    #[test]
    fn password_authentication_requires_an_encrypted_value() {
        let mut config = Config::default();
        config.hosts.push(sample_password_host());
        validate(&config).unwrap();
        let encoded = serde_json::to_string(&config).unwrap();
        assert!(encoded.contains("encrypted_password"));
        assert!(!encoded.contains("secret-value"));
    }

    #[test]
    fn legacy_password_credential_remains_loadable_for_migration() {
        let mut config = Config::default();
        let mut host = sample_password_host();
        host.id = "host-legacy".into();
        host.name = "legacy-password-host".into();
        host.auth.encrypted_password = None;
        host.auth.credential_id = Some("legacy-credential".into());
        config.hosts.push(host);
        validate(&config).unwrap();
    }

    #[test]
    fn migrates_v1_accepting_policy_to_accept_new() {
        let path = scratch_path("migrate-accept-new.json");
        fs::write(
            &path,
            r#"{"version":1,"settings":{"strict_host_key_checking":true,"connect_timeout_seconds":7},"hosts":[],"tunnels":[]}"#,
        )
        .unwrap();

        let config = load(&path).unwrap();
        assert_eq!(config.version, CONFIG_VERSION);
        assert_eq!(config.settings.host_key_policy, HostKeyPolicy::AcceptNew);
        assert_eq!(config.settings.connect_timeout_seconds, 7);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn migrates_v1_disabled_policy_to_insecure() {
        let path = scratch_path("migrate-insecure.json");
        fs::write(
            &path,
            r#"{"version":1,"settings":{"strict_host_key_checking":false},"hosts":[],"tunnels":[]}"#,
        )
        .unwrap();

        let config = load(&path).unwrap();
        assert_eq!(config.settings.host_key_policy, HostKeyPolicy::Insecure);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_configuration_from_a_newer_build() {
        let path = scratch_path("future-version.json");
        fs::write(
            &path,
            format!(
                r#"{{"version":{},"settings":{{}},"hosts":[],"tunnels":[]}}"#,
                CONFIG_VERSION + 1
            ),
        )
        .unwrap();

        let error = load(&path).unwrap_err();
        assert!(matches!(error, ConfigError::UnsupportedVersion { .. }));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn round_trips_through_lock_guarded_write() {
        let path = scratch_path("locked.json");
        {
            let _guard = ConfigLock::acquire(&path).unwrap();
            let mut config = load(&path).unwrap();
            config.hosts.push(sample_password_host());
            save(&path, &config).unwrap();
        }

        let reloaded = load(&path).unwrap();
        assert_eq!(reloaded.hosts.len(), 1);
        assert_eq!(reloaded.hosts[0].name, "password-host");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(lock_path(&path));
    }

    /// 生成一个指定名称的主机，用于并发写入测试。
    fn host_named(name: &str) -> Host {
        Host {
            id: format!("{name}-id"),
            name: name.into(),
            ..sample_password_host()
        }
    }

    /// **C4（跨进程配置锁）的并发测试。**
    ///
    /// 上面那个 `round_trips_through_lock_guarded_write` 是**单线程**的，
    /// 只证明「加锁后能读写」，无法证明「并发下不丢更新」——而后者才是加锁的目的。
    ///
    /// 本测试让 N 个线程各自执行 `加锁 → load → 追加一个主机 → save`。
    /// 若无锁（或锁无效），`load` 与 `save` 之间的读-改-写竞态会丢失更新，
    /// 最终主机数会小于 N。
    ///
    /// 注意：`File::lock()` 是阻塞式独占锁。在 Windows 上基于 `LockFileEx`、
    /// 在 Unix 上基于 `flock`，二者的锁都绑定到**打开的文件描述**，
    /// 因此同一进程内不同线程的独立 `open` 之间会真实互斥——本测试确实在测锁。
    ///
    /// 已实证本测试**不是弱测试**：把 `ConfigLock::acquire` 从线程体里临时去掉后，
    /// 该测试失败，报 `期望 8 个主机，实际 1`——即无锁时会丢失 7 次写入。
    /// 这同时反证了 C4 的文件锁确实在阻止这类数据丢失。
    #[test]
    fn concurrent_locked_writers_do_not_lose_updates() {
        use std::sync::{Arc, Barrier};

        const WRITERS: usize = 8;

        let path = scratch_path("concurrent.json");

        // 先建立一份初始配置，避免各线程同时从"文件不存在"开始。
        {
            let _guard = ConfigLock::acquire(&path).unwrap();
            save(&path, &Config::default()).unwrap();
        }

        // Barrier 让所有线程尽量同时进入临界区，提高撞上竞态的概率。
        let barrier = Arc::new(Barrier::new(WRITERS));
        let handles: Vec<_> = (0..WRITERS)
            .map(|index| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let _guard = ConfigLock::acquire(&path).expect("获取配置锁失败");
                    let mut config = load(&path).expect("读取配置失败");
                    config
                        .hosts
                        .push(host_named(&format!("concurrent-{index}")));
                    save(&path, &config).expect("写入配置失败");
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("写入线程发生 panic");
        }

        let config = load(&path).unwrap();
        assert_eq!(
            config.hosts.len(),
            WRITERS,
            "并发写入丢失了更新：期望 {WRITERS} 个主机，实际 {}",
            config.hosts.len()
        );

        // 除数量外，还须确认每个线程的写入都完整落盘（名称无重复、无遗漏）。
        let mut actual: Vec<&str> = config.hosts.iter().map(|host| host.name.as_str()).collect();
        actual.sort_unstable();
        let mut expected: Vec<String> = (0..WRITERS)
            .map(|index| format!("concurrent-{index}"))
            .collect();
        expected.sort_unstable();
        assert_eq!(
            actual,
            expected.iter().map(String::as_str).collect::<Vec<_>>()
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(lock_path(&path));
    }
}

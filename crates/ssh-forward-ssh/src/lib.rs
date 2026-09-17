use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, ChildStderr, Command, Stdio},
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

use ssh_forward_config::{AuthType, Host, Settings, Tunnel};
use thiserror::Error;

/// 每个 Tunnel 保留的 OpenSSH stderr 行数上限。
const STDERR_TAIL_LIMIT: usize = 200;

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

/// 启动一次转发所需的全部输入。
///
/// 用结构体代替长参数列表，避免调用点参数错位，也方便后续继续扩展字段。
pub struct ForwardSpec<'a> {
    pub settings: &'a Settings,
    pub host: &'a Host,
    pub tunnel: &'a Tunnel,
    pub jump_host: Option<&'a Host>,
    /// 密码认证的明文口令；`None` 表示使用 SSH Agent 或私钥。
    pub password: Option<&'a str>,
    /// 应用私有 known_hosts 文件。传 `None` 会退回 OpenSSH 默认行为，
    /// 即读写用户主目录下的 `~/.ssh/known_hosts`，通常不应这么做。
    pub known_hosts: Option<&'a Path>,
    /// 诊断日志落盘路径。`None` 表示只保留内存中的 stderr 尾部。
    pub log_path: Option<&'a Path>,
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

pub fn openssh_arguments(spec: &ForwardSpec<'_>) -> Vec<String> {
    let settings = spec.settings;
    let host = spec.host;
    let tunnel = spec.tunnel;

    let mut arguments = vec![
        "-N".into(),
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={}", settings.connect_timeout_seconds),
        "-o".into(),
        format!(
            "StrictHostKeyChecking={}",
            settings.host_key_policy.openssh_value()
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

    // 把可信主机记录限制在应用私有文件内，避免污染用户自己的 ~/.ssh/known_hosts。
    if let Some(path) = spec.known_hosts {
        arguments.extend([
            "-o".into(),
            format!("UserKnownHostsFile={}", path.display()),
        ]);
    }

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

    if let Some(jump) = spec.jump_host {
        arguments.extend([
            "-J".into(),
            format!("{}@{}:{}", jump.username, jump.hostname, jump.port),
        ]);
    }

    // 自定义 -o 选项（主机级与隧道级）。
    //
    // 注意：OpenSSH 对同名参数采用「首个取值生效」，内置选项排在前面，
    // 因此用户自定义的同名选项会被静默忽略，而不是覆盖安全默认值。
    for opt in host
        .custom_options
        .iter()
        .chain(tunnel.custom_options.iter())
    {
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

/// 运行期诊断信息：保留 OpenSSH stderr 的尾部若干行，并可选落盘。
#[derive(Default)]
struct Diagnostics {
    tail: VecDeque<String>,
    log: Option<std::fs::File>,
}

impl Diagnostics {
    fn record(&mut self, line: &str) {
        if self.tail.len() == STDERR_TAIL_LIMIT {
            self.tail.pop_front();
        }
        self.tail.push_back(line.to_owned());
        if let Some(log) = self.log.as_mut() {
            let _ = writeln!(log, "{line}");
        }
    }
}

pub struct OpenSshForward {
    child: Child,
    askpass: Option<PathBuf>,
    diagnostics: Arc<Mutex<Diagnostics>>,
    stderr_reader: Option<JoinHandle<()>>,
    log_path: Option<PathBuf>,
}

impl OpenSshForward {
    pub fn start(spec: &ForwardSpec<'_>) -> Result<Self, SshError> {
        check_local_port(spec.tunnel)?;

        let diagnostics = Arc::new(Mutex::new(Diagnostics {
            tail: VecDeque::new(),
            log: spec.log_path.and_then(open_log),
        }));
        if let Ok(mut guard) = diagnostics.lock() {
            guard.record(&format!(
                "--- start tunnel={} type={:?} target={}@{}:{} ---",
                spec.tunnel.name,
                spec.tunnel.kind,
                spec.host.username,
                spec.host.hostname,
                spec.host.port
            ));
        }

        let askpass = spec.password.map(|_| create_askpass_script()).transpose()?;
        let mut command = Command::new("ssh");
        command
            .args(openssh_arguments(spec))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;

            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        if let Some(password) = spec.password {
            command.env("SSH_ASKPASS", askpass.as_ref().expect("askpass exists"));
            command.env("SSH_ASKPASS_REQUIRE", "force");
            command.env("DISPLAY", "ssh-forward");
            command.env("SSH_FORWARD_PASSWORD", password);
        }
        let mut child = command.spawn().map_err(SshError::Start)?;
        #[cfg(windows)]
        {
            job::assign_to_job(&child);
        }

        let stderr_reader = child
            .stderr
            .take()
            .map(|stderr| spawn_stderr_reader(stderr, Arc::clone(&diagnostics)));

        Ok(Self {
            child,
            askpass,
            diagnostics,
            stderr_reader,
            log_path: spec.log_path.map(Path::to_path_buf),
        })
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn is_running(&mut self) -> Result<bool, SshError> {
        Ok(self.child.try_wait().map_err(SshError::Start)?.is_none())
    }

    /// 最近若干行 OpenSSH stderr，按时间顺序排列。
    pub fn stderr_tail(&self) -> Vec<String> {
        self.diagnostics
            .lock()
            .map(|guard| guard.tail.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// 供界面展示的诊断文本；无输出时返回空字符串。
    pub fn diagnostics_text(&self) -> String {
        self.stderr_tail().join("\n")
    }

    pub fn log_path(&self) -> Option<&Path> {
        self.log_path.as_deref()
    }

    pub fn stop(&mut self) -> Result<(), SshError> {
        if self.child.try_wait().map_err(SshError::Start)?.is_none() {
            self.child.kill().map_err(SshError::Start)?;
            self.child.wait().map_err(SshError::Start)?;
        }
        self.join_reader();
        Ok(())
    }

    fn join_reader(&mut self) {
        if let Some(handle) = self.stderr_reader.take() {
            // 子进程已结束，stderr 管道随之关闭，读取线程会立刻返回。
            let _ = handle.join();
        }
    }
}

impl Drop for OpenSshForward {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.join_reader();
        if let Some(path) = &self.askpass {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn open_log(path: &Path) -> Option<std::fs::File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).ok()?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

fn spawn_stderr_reader(
    stderr: ChildStderr,
    diagnostics: Arc<Mutex<Diagnostics>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            match diagnostics.lock() {
                Ok(mut guard) => guard.record(&line),
                Err(_) => break,
            }
        }
    })
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
    use ssh_forward_config::{Auth, Endpoint, HostKeyPolicy, TunnelType};

    fn spec<'a>(
        settings: &'a Settings,
        host: &'a Host,
        tunnel: &'a Tunnel,
        jump_host: Option<&'a Host>,
    ) -> ForwardSpec<'a> {
        ForwardSpec {
            settings,
            host,
            tunnel,
            jump_host,
            password: None,
            known_hosts: None,
            log_path: None,
        }
    }

    fn sample_host() -> Host {
        Host {
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
        }
    }

    fn sample_tunnel(host_id: &str) -> Tunnel {
        Tunnel {
            id: "tunnel-1".into(),
            name: "web".into(),
            host_id: host_id.into(),
            kind: TunnelType::Local,
            local: Endpoint::localhost(18080),
            remote: Some(Endpoint::localhost(8080)),
            gateway_ports: true,
            custom_options: vec![],
            auto_start: false,
            auto_reconnect: true,
            auto_open_browser: false,
            enabled: true,
        }
    }

    #[test]
    fn builds_secure_local_forward_command() {
        let settings = Settings::default();
        let host = sample_host();
        let tunnel = sample_tunnel(&host.id);
        let args = openssh_arguments(&spec(&settings, &host, &tunnel, None));

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
    fn maps_every_host_key_policy_to_its_openssh_value() {
        let host = sample_host();
        let tunnel = sample_tunnel(&host.id);

        for (policy, expected) in [
            (HostKeyPolicy::AcceptNew, "StrictHostKeyChecking=accept-new"),
            (HostKeyPolicy::Strict, "StrictHostKeyChecking=yes"),
            (HostKeyPolicy::Insecure, "StrictHostKeyChecking=no"),
        ] {
            let settings = Settings {
                host_key_policy: policy,
                ..Settings::default()
            };
            let args = openssh_arguments(&spec(&settings, &host, &tunnel, None));
            assert!(args.contains(&expected.into()), "policy {policy:?}");
        }
    }

    #[test]
    fn isolates_known_hosts_when_a_private_path_is_given() {
        let settings = Settings::default();
        let host = sample_host();
        let tunnel = sample_tunnel(&host.id);
        let known_hosts = Path::new("/tmp/app-private/known_hosts");

        let mut request = spec(&settings, &host, &tunnel, None);
        request.known_hosts = Some(known_hosts);
        let args = openssh_arguments(&request);

        assert!(
            args.contains(&format!("UserKnownHostsFile={}", known_hosts.display())),
            "expected private known_hosts to be passed through"
        );
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
        let mut tunnel = sample_tunnel(&host.id);
        tunnel.id = "tunnel-dyn".into();
        tunnel.name = "socks5".into();
        tunnel.kind = TunnelType::Dynamic;
        tunnel.local = Endpoint::localhost(10808);
        tunnel.remote = None;
        tunnel.gateway_ports = false;

        let args = openssh_arguments(&spec(&settings, &host, &tunnel, Some(&jump)));
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

    // ── 以下为 F3（补充测试覆盖）新增：覆盖此前完全无测试的边界 ──────────────

    /// 远端转发（`-R`）此前**完全没有测试覆盖**。
    ///
    /// OpenSSH 的 `-R` 语法是 `[bind:]port:host:hostport`，即**先监听端、后目标端**，
    /// 与 `-L` 的书写顺序正好相反。本测试固化该顺序，避免日后被误改成 `-L` 的顺序。
    #[test]
    fn builds_remote_forward_command_with_listening_side_first() {
        let settings = Settings::default();
        let host = sample_host();
        let mut tunnel = sample_tunnel(&host.id);
        tunnel.kind = TunnelType::Remote;
        tunnel.local = Endpoint::localhost(9000);
        tunnel.remote = Some(Endpoint {
            host: "public.example".into(),
            port: 443,
        });

        let args = openssh_arguments(&spec(&settings, &host, &tunnel, None));

        assert!(
            args.windows(2)
                .any(|pair| pair == ["-R", "public.example:443:127.0.0.1:9000"]),
            "-R 必须把远端监听端写在前面，实际参数为 {args:?}"
        );
    }

    /// `remote` 缺失时，Local 与 Remote 转发都不会产生任何 `-L` / `-R`。
    ///
    /// 即：会启动一个**什么都不转发**的 SSH 会话，且不会有任何报错。
    /// 本测试把该行为显式固化下来——它意味着配置层必须保证 `remote` 存在，
    /// 否则用户会看到一个"已运行但不工作"的转发。
    #[test]
    fn omits_forward_flag_when_remote_endpoint_is_missing() {
        let settings = Settings::default();
        let host = sample_host();

        for kind in [TunnelType::Local, TunnelType::Remote] {
            let mut tunnel = sample_tunnel(&host.id);
            tunnel.kind = kind.clone();
            tunnel.remote = None;

            let args = openssh_arguments(&spec(&settings, &host, &tunnel, None));

            assert!(
                !args.iter().any(|a| a == "-L" || a == "-R"),
                "{kind:?} 在缺少 remote 时不应产生转发参数，实际为 {args:?}"
            );
            // 但会话本身仍然会被启动（目标主机参数照常追加）。
            assert!(args.contains(&"alice@example.test".into()));
        }
    }

    /// **D6 的证据测试**：自定义 `-o` 无法覆盖内置安全项。
    ///
    /// OpenSSH 对同名 `-o` 采用「**首个取值生效**」。本实现把内置项排在前面、
    /// 自定义项排在后面，因此用户写的 `StrictHostKeyChecking=no` 会被**静默忽略**，
    /// 而用户会以为它生效了。
    ///
    /// 本测试固化的是**当前（有问题的）行为**，作为修复 D6 的基线：
    /// 修复后（例如改为显式提示冲突、或拒绝同名项）该断言应当随之改变。
    #[test]
    fn custom_options_are_placed_after_builtins_and_therefore_cannot_override_them() {
        let settings = Settings::default();
        let mut host = sample_host();
        host.custom_options = vec!["StrictHostKeyChecking=no".into()];
        let tunnel = sample_tunnel(&host.id);

        let args = openssh_arguments(&spec(&settings, &host, &tunnel, None));

        let builtin = args
            .iter()
            .position(|a| a == "StrictHostKeyChecking=accept-new")
            .expect("内置的主机密钥策略应当存在");
        let custom = args
            .iter()
            .position(|a| a == "StrictHostKeyChecking=no")
            .expect("用户自定义项应当被原样追加");

        assert!(
            builtin < custom,
            "内置项必须排在自定义项之前（OpenSSH 取首个值），实际 builtin={builtin} custom={custom}"
        );
    }

    /// 主机级自定义项排在隧道级之前；同名时主机级生效。
    #[test]
    fn orders_host_custom_options_before_tunnel_custom_options() {
        let settings = Settings::default();
        let mut host = sample_host();
        host.custom_options = vec!["Compression=yes".into()];
        let mut tunnel = sample_tunnel(&host.id);
        tunnel.custom_options = vec!["Compression=no".into()];

        let args = openssh_arguments(&spec(&settings, &host, &tunnel, None));

        let host_level = args
            .iter()
            .position(|a| a == "Compression=yes")
            .expect("主机级自定义项应存在");
        let tunnel_level = args
            .iter()
            .position(|a| a == "Compression=no")
            .expect("隧道级自定义项应存在");

        assert!(
            host_level < tunnel_level,
            "主机级应排在隧道级之前，实际 host={host_level} tunnel={tunnel_level}"
        );
    }

    /// 空串与纯空白项会被跳过，不会产生缺少取值的 `-o`。
    #[test]
    fn skips_blank_custom_options() {
        let settings = Settings::default();
        let mut host = sample_host();
        host.custom_options = vec!["".into(), "   ".into(), "\t".into(), "  Valid=1  ".into()];
        let tunnel = sample_tunnel(&host.id);

        let args = openssh_arguments(&spec(&settings, &host, &tunnel, None));

        // 有效项应被 trim 后追加。
        assert!(args.contains(&"Valid=1".into()));
        // 空白项不应产生任何空的 -o 取值。
        assert!(
            !args.iter().any(|a| a.is_empty()),
            "不应出现空参数，实际为 {args:?}"
        );
        // "-o" 的次数应等于有效选项数（内置 6 项 + 1 个自定义项）。
        let option_flag_count = args.iter().filter(|a| *a == "-o").count();
        assert_eq!(
            option_flag_count, 7,
            "内置 6 项 + 1 个有效自定义项，实际为 {option_flag_count}"
        );
    }

    /// 证书与 ProxyCommand 只在非空时追加，空串不应产生无效参数。
    #[test]
    fn adds_certificate_and_proxy_command_only_when_non_empty() {
        let settings = Settings::default();

        let mut host = sample_host();
        host.certificate_file = Some("/keys/id_ed25519-cert.pub".into());
        host.proxy_command = Some("nc %h %p".into());
        let args = openssh_arguments(&spec(&settings, &host, &sample_tunnel(&host.id), None));
        assert!(args.contains(&"CertificateFile=/keys/id_ed25519-cert.pub".into()));
        assert!(args.contains(&"ProxyCommand=nc %h %p".into()));

        let mut blank = sample_host();
        blank.certificate_file = Some("   ".into());
        blank.proxy_command = Some("".into());
        let args = openssh_arguments(&spec(&settings, &blank, &sample_tunnel(&blank.id), None));
        assert!(
            !args.iter().any(|a| a.starts_with("CertificateFile=")),
            "空白证书路径不应产生参数"
        );
        assert!(
            !args.iter().any(|a| a.starts_with("ProxyCommand=")),
            "空白 ProxyCommand 不应产生参数"
        );
    }

    /// 私钥认证默认追加 `IdentitiesOnly=yes`，但可被显式设为 `false` 关闭。
    #[test]
    fn requests_identities_only_for_private_key_auth_unless_explicitly_disabled() {
        let settings = Settings::default();

        // 未显式设置：私钥认证默认开启。
        let mut host = sample_host();
        host.auth = Auth {
            kind: AuthType::PrivateKey,
            private_key: Some("/keys/id_ed25519".into()),
            ..Auth::default()
        };
        let args = openssh_arguments(&spec(&settings, &host, &sample_tunnel(&host.id), None));
        assert!(args.contains(&"IdentitiesOnly=yes".into()));
        assert!(args.windows(2).any(|p| p == ["-i", "/keys/id_ed25519"]));

        // 显式关闭：不再追加。
        host.identities_only = Some(false);
        let args = openssh_arguments(&spec(&settings, &host, &sample_tunnel(&host.id), None));
        assert!(
            !args.contains(&"IdentitiesOnly=yes".into()),
            "显式关闭后不应再追加 IdentitiesOnly"
        );

        // SSH Agent 认证且未显式设置：不追加。
        let mut agent = sample_host();
        agent.auth = Auth::default();
        let args = openssh_arguments(&spec(&settings, &agent, &sample_tunnel(&agent.id), None));
        assert!(!args.contains(&"IdentitiesOnly=yes".into()));
    }

    /// 主机端口通过 `-p` 传递，且目标主机参数位于参数表末尾。
    #[test]
    fn appends_port_and_destination_last() {
        let settings = Settings::default();
        let host = sample_host();
        let tunnel = sample_tunnel(&host.id);

        let args = openssh_arguments(&spec(&settings, &host, &tunnel, None));

        assert!(args.windows(2).any(|p| p == ["-p", "2222"]));
        assert_eq!(args.last().map(String::as_str), Some("alice@example.test"));
        // 第一条必须是 -N（不执行远程命令，纯转发）。
        assert_eq!(args.first().map(String::as_str), Some("-N"));
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

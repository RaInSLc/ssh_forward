use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Result, anyhow};
use clap::{Args, Parser, Subcommand};
use ssh_forward_config::{Config, Endpoint, load, validate};
use ssh_forward_core::{
    AuthInput, HostDraft, RuntimePaths, TunnelDraft, remove_host, remove_tunnel, start_tunnel,
    upsert_host, upsert_tunnel,
};

#[derive(Debug, Parser)]
#[command(
    name = "ssh-forward",
    version,
    about = "Manage secure local SSH port forwards"
)]
struct Cli {
    #[arg(
        long,
        global = true,
        default_value = "config.json",
        help = "Path to the JSON configuration file"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Host {
        #[command(subcommand)]
        command: HostCommand,
    },
    Tunnel {
        #[command(subcommand)]
        command: TunnelCommand,
    },
    Start {
        tunnel: String,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Path,
    Validate,
    Show,
}

#[derive(Debug, Subcommand)]
enum HostCommand {
    List,
    Add(HostAdd),
    Remove { name: String },
}

#[derive(Debug, Args)]
struct HostAdd {
    name: String,
    #[arg(long)]
    host: String,
    #[arg(long)]
    user: String,
    #[arg(long, default_value_t = 22)]
    port: u16,
    #[arg(long, help = "Private key path. Omit to use SSH Agent.")]
    key: Option<String>,
}

#[derive(Debug, Subcommand)]
enum TunnelCommand {
    List,
    Add(TunnelAdd),
    Remove { name: String },
}

#[derive(Debug, Args)]
struct TunnelAdd {
    name: String,
    #[arg(long)]
    host: String,
    #[arg(long, value_parser = parse_endpoint, help = "Loopback bind endpoint, for example 127.0.0.1:18888")]
    local: Endpoint,
    #[arg(long, value_parser = parse_endpoint, help = "Remote endpoint, for example 127.0.0.1:8888")]
    remote: Endpoint,
}

fn parse_endpoint(value: &str) -> Result<Endpoint, String> {
    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| "endpoint must be HOST:PORT".to_owned())?;
    let port = port
        .parse::<u16>()
        .map_err(|_| "port must be an integer in range 1..=65535".to_owned())?;
    if host.trim().is_empty() || port == 0 {
        return Err("endpoint must contain a host and a port in range 1..=65535".into());
    }
    Ok(Endpoint {
        host: host.into(),
        port,
    })
}

/// 解析一条隧道的运行期路径（应用私有 `known_hosts` + 诊断日志）。
///
/// 单独抽出以便直接测试。此处曾长期把 `StartOptions::default()` 交给 core，三个字段
/// 全空，于是 OpenSSH 退回默认行为：把信任记录写进**用户主目录的 `~/.ssh/known_hosts`**
/// （污染系统级文件），并且不产生任何诊断日志。
///
/// 路径推导本身由 [`RuntimePaths`] 提供，与桌面壳层共用同一实现。
fn runtime_paths_for(
    config: &Config,
    config_path: &Path,
    tunnel_name: &str,
) -> Result<RuntimePaths> {
    let tunnel = config
        .tunnels
        .iter()
        .find(|tunnel| tunnel.name == tunnel_name)
        .ok_or_else(|| anyhow!("未找到 Tunnel '{tunnel_name}'"))?;
    Ok(RuntimePaths::for_tunnel(config_path, &tunnel.id))
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Config {
            command: ConfigCommand::Path,
        } => println!("{}", cli.config.display()),
        Command::Config {
            command: ConfigCommand::Validate,
        } => {
            let config = load(&cli.config)?;
            validate(&config)?;
            println!("Configuration is valid: {}", cli.config.display());
        }
        Command::Config {
            command: ConfigCommand::Show,
        } => {
            let config = load(&cli.config)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        Command::Host {
            command: HostCommand::List,
        } => {
            let config = load(&cli.config)?;
            println!("NAME\tHOST\tUSER\tPORT\tAUTH");
            for host in config.hosts {
                println!(
                    "{}\t{}\t{}\t{}\t{:?}",
                    host.name, host.hostname, host.username, host.port, host.auth.kind
                );
            }
        }
        Command::Host {
            command: HostCommand::Add(arguments),
        } => {
            let auth = match arguments.key {
                Some(path) => AuthInput::PrivateKey { path },
                None => AuthInput::SshAgent,
            };
            let host = upsert_host(
                &cli.config,
                None,
                HostDraft {
                    name: arguments.name,
                    hostname: arguments.host,
                    port: arguments.port,
                    username: arguments.user,
                    auth,
                    jump_host_id: None,
                    proxy_command: None,
                    identities_only: None,
                    certificate_file: None,
                    compression: None,
                    custom_options: Vec::new(),
                    enabled: true,
                },
            )?;
            println!("Added host '{}' ({})", host.name, host.id);
        }
        Command::Host {
            command: HostCommand::Remove { name },
        } => {
            remove_host(&cli.config, &name)?;
            println!("Removed host '{name}'");
        }
        Command::Tunnel {
            command: TunnelCommand::List,
        } => {
            let config = load(&cli.config)?;
            println!("NAME\tHOST\tLOCAL\tREMOTE\tTYPE");
            for tunnel in config.tunnels {
                let host = config
                    .hosts
                    .iter()
                    .find(|host| host.id == tunnel.host_id)
                    .map(|host| host.name.as_str())
                    .unwrap_or("<missing>");
                let remote_str = tunnel
                    .remote
                    .as_ref()
                    .map(|r| format!("{}:{}", r.host, r.port))
                    .unwrap_or_else(|| "-".into());
                println!(
                    "{}\t{}\t{}:{}\t{}\t{:?}",
                    tunnel.name,
                    host,
                    tunnel.local.host,
                    tunnel.local.port,
                    remote_str,
                    tunnel.kind
                );
            }
        }
        Command::Tunnel {
            command: TunnelCommand::Add(arguments),
        } => {
            let tunnel = upsert_tunnel(
                &cli.config,
                None,
                TunnelDraft {
                    name: arguments.name,
                    host_name: arguments.host,
                    kind: ssh_forward_config::TunnelType::Local,
                    local: arguments.local,
                    remote: Some(arguments.remote),
                    gateway_ports: false,
                    custom_options: Vec::new(),
                    auto_open_browser: false,
                },
            )?;
            println!("Added tunnel '{}' ({})", tunnel.name, tunnel.id);
        }
        Command::Tunnel {
            command: TunnelCommand::Remove { name },
        } => {
            remove_tunnel(&cli.config, &name)?;
            println!("Removed tunnel '{name}'");
        }
        Command::Start { tunnel } => {
            let config = load(&cli.config)?;
            let paths = runtime_paths_for(&config, &cli.config, &tunnel)?;
            let forward = start_tunnel(&config, &tunnel, paths.options(None))?;
            println!(
                "Tunnel '{tunnel}' started with OpenSSH process {}. Press Ctrl+C to stop.",
                forward.id()
            );
            // 日志路径由 CLI 推导，用户无从猜测，因此显式告知。
            println!("Diagnostics: {}", paths.log_path().display());
            wait_for_interrupt();
            drop(forward);
        }
    }
    Ok(())
}

/// 阻塞当前线程，把生命周期交给 Ctrl+C。
///
/// 使用 `park` 而不是 sleep 轮询，避免空转占用 CPU。
fn wait_for_interrupt() {
    loop {
        std::thread::park();
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssh_forward_config::{Auth, AuthType, Host, Tunnel, TunnelType};

    const TUNNEL_ID: &str = "11111111-1111-1111-1111-111111111111";
    const CONFIG_PATH: &str = "E:/tmp/ssh-forward-cli-fixture/config.json";

    fn config_with_tunnel(name: &str) -> Config {
        let mut config = Config::default();
        config.hosts.push(Host {
            id: "host-1".into(),
            name: "example".into(),
            hostname: "example.test".into(),
            port: 22,
            username: "alice".into(),
            auth: Auth {
                kind: AuthType::SshAgent,
                private_key: None,
                credential_id: None,
                encrypted_password: None,
            },
            jump_host_id: None,
            proxy_command: None,
            identities_only: None,
            certificate_file: None,
            compression: None,
            custom_options: Vec::new(),
            enabled: true,
        });
        config.tunnels.push(Tunnel {
            id: TUNNEL_ID.into(),
            name: name.into(),
            host_id: "host-1".into(),
            kind: TunnelType::Local,
            local: Endpoint::localhost(13306),
            remote: Some(Endpoint::localhost(3306)),
            gateway_ports: false,
            custom_options: Vec::new(),
            auto_start: false,
            auto_reconnect: true,
            auto_open_browser: false,
            enabled: true,
        });
        config
    }

    /// **污染回归测试**：CLI 启动隧道必须传入应用私有 `known_hosts` 与日志路径。
    ///
    /// 修复前 `Start` 分支传的是 `StartOptions::default()`——三个字段全空，
    /// 于是 OpenSSH 把信任记录写进用户主目录的 `~/.ssh/known_hosts`，
    /// 并且隧道完全没有诊断日志。
    #[test]
    fn start_options_isolate_known_hosts_and_enable_diagnostics() {
        let config = config_with_tunnel("database");

        let paths = runtime_paths_for(&config, Path::new(CONFIG_PATH), "database")
            .expect("应能解析已存在的隧道");

        assert_eq!(
            paths.known_hosts(),
            Path::new("E:/tmp/ssh-forward-cli-fixture/known_hosts")
        );
        assert!(
            !paths.known_hosts().to_string_lossy().contains(".ssh"),
            "CLI 不得使用 ~/.ssh/known_hosts，实际 {}",
            paths.known_hosts().display()
        );

        let expected_log = format!("E:/tmp/ssh-forward-cli-fixture/logs/tunnel-{TUNNEL_ID}.log");
        assert_eq!(paths.log_path(), Path::new(&expected_log));

        // 修复的落点是把这两个路径真正交给 core，而不只是推导出来。
        let options = paths.options(None);
        assert!(
            options.known_hosts.is_some(),
            "必须传入应用私有 known_hosts"
        );
        assert!(options.log_path.is_some(), "必须启用诊断日志");
    }

    /// 负向用例：隧道名不存在时必须报错，而不是静默退回默认（无路径）选项。
    #[test]
    fn runtime_paths_for_rejects_an_unknown_tunnel() {
        let config = config_with_tunnel("database");

        let error = runtime_paths_for(&config, Path::new(CONFIG_PATH), "missing")
            .expect_err("未知隧道名必须报错");

        assert!(
            error.to_string().contains("missing"),
            "错误信息应指出找不到的隧道名，实际 {error}"
        );
    }
}

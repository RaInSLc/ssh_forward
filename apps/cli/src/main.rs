use std::{path::PathBuf, process::ExitCode};

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use ssh_forward_config::{Endpoint, load, validate};
use ssh_forward_core::{add_host, add_tunnel, remove_host, remove_tunnel, start_tunnel};

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
            let host = add_host(
                &cli.config,
                arguments.name,
                arguments.host,
                arguments.port,
                arguments.user,
                arguments.key,
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
                println!(
                    "{}\t{}\t{}:{}\t{}:{}\t{:?}",
                    tunnel.name,
                    host,
                    tunnel.local.host,
                    tunnel.local.port,
                    tunnel.remote.host,
                    tunnel.remote.port,
                    tunnel.kind
                );
            }
        }
        Command::Tunnel {
            command: TunnelCommand::Add(arguments),
        } => {
            let tunnel = add_tunnel(
                &cli.config,
                arguments.name,
                &arguments.host,
                arguments.local,
                arguments.remote,
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
            let forward = start_tunnel(&config, &tunnel)?;
            println!(
                "Tunnel '{tunnel}' started with OpenSSH process {}. Press Ctrl+C to stop.",
                forward.id()
            );
            wait_for_interrupt()?;
            drop(forward);
        }
    }
    Ok(())
}

fn wait_for_interrupt() -> Result<()> {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
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

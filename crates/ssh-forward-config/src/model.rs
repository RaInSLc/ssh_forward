use serde::{Deserialize, Serialize};

use crate::CONFIG_VERSION;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub tunnels: Vec<Tunnel>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            settings: Settings::default(),
            hosts: Vec::new(),
            tunnels: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_true")]
    pub strict_host_key_checking: bool,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_seconds: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            strict_host_key_checking: true,
            connect_timeout_seconds: default_connect_timeout(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_connect_timeout() -> u16 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub hostname: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_ssh_port() -> u16 {
    22
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Auth {
    #[serde(rename = "type", default)]
    pub kind: AuthType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_password: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    #[default]
    SshAgent,
    PrivateKey,
    Password,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tunnel {
    pub id: String,
    pub name: String,
    pub host_id: String,
    #[serde(rename = "type")]
    pub kind: TunnelType,
    pub local: Endpoint,
    pub remote: Endpoint,
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TunnelType {
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    pub fn localhost(port: u16) -> Self {
        Self {
            host: "127.0.0.1".into(),
            port,
        }
    }
}

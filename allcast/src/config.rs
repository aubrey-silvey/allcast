//! Persistent config — loaded on launch, written by the egui first-run window.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const APP_DIR: &str = "allcast";
pub const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Sender,
    Receiver,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    H265,
}

/// Two presets that bundle framerate/bitrate/encoder preset into a single
/// choice. Text optimises for low latency on static screens; Video for
/// smooth motion at the cost of latency.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Text,
    Video,
}

impl Quality {
    pub fn framerate(self) -> u32 {
        match self { Quality::Text => 30, Quality::Video => 60 }
    }
    pub fn bitrate_kbps(self) -> u32 {
        match self { Quality::Text => 12_000, Quality::Video => 20_000 }
    }
    pub fn target_usage(self) -> u32 {
        // 2 = balanced+quality on VA-API. Latency-equivalent to higher values
        // in our measurements; quality is better.
        2
    }
    pub fn rate_control(self) -> &'static str {
        "cbr"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    /// User-friendly label, optional.
    #[serde(default)]
    pub label: String,
    /// host:port the sender pushes RTP to.
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub role: Role,
    pub quality: Quality,
    pub codec: Codec,
    pub payload_type: u8,

    /// Saved peer destinations. Sender pushes to `peers[active_peer]`.
    #[serde(default)]
    pub peers: Vec<Peer>,
    #[serde(default)]
    pub active_peer: usize,

    /// Receiver listens here. Sender ignores.
    pub listen_port: u16,

    pub width: u32,
    pub height: u32,

    /// Identifier of the monitor — interpretation is OS- and role-specific.
    pub monitor: String,

    /// receiver-only tuning
    pub jitter_ms: u32,
    pub idle_timeout_s: u32,
    pub rcvbuf: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            role: Role::Receiver,
            quality: Quality::Text,
            codec: Codec::H265,
            payload_type: 96,
            peers: Vec::new(),
            active_peer: 0,
            listen_port: 5004,
            width: 1920,
            height: 1080,
            monitor: "auto".into(),
            jitter_ms: 10,
            idle_timeout_s: 3,
            rcvbuf: 8 * 1024 * 1024,
        }
    }
}

impl Config {
    pub fn active_peer_address(&self) -> Option<&str> {
        self.peers
            .get(self.active_peer)
            .map(|p| p.address.as_str())
    }
}

pub fn config_dir() -> Result<PathBuf> {
    let d = dirs::config_dir().ok_or_else(|| anyhow!("no platform config directory"))?;
    Ok(d.join(APP_DIR))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(CONFIG_FILE))
}

pub fn exists() -> bool {
    config_path().map(|p| p.is_file()).unwrap_or(false)
}

pub fn load() -> Result<Config> {
    let path = config_path()?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("reading config at {}", path.display()))?;
    let cfg: Config = toml::from_str(&raw)
        .with_context(|| format!("parsing config at {}", path.display()))?;
    Ok(cfg)
}

pub fn save(cfg: &Config) -> Result<()> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = config_path()?;
    let body = toml::to_string_pretty(cfg).context("serializing config")?;
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

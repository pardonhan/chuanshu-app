use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::SystemTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperatingSystem {
    Windows,
    MacOS,
    Linux,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Capability {
    FolderTransfer,
    ResumeTransfer,
    MultiDeviceSend,
    P2PTransfer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub device_id: Uuid,
    pub device_name: String,
    pub os: OperatingSystem,
    pub ip_address: IpAddr,
    pub quic_port: u16,
    pub protocol_version: String,
    pub capabilities: Vec<Capability>,
    pub last_seen: SystemTime,
    /// SHA-256 fingerprint of the device's QUIC certificate (hex, lowercase).
    pub cert_fingerprint: String,
}

impl DeviceInfo {
    pub fn is_online(&self) -> bool {
        self.last_seen
            .elapsed()
            .map(|d| d.as_secs() < crate::core::constants::DISCOVERY_TIMEOUT)
            .unwrap_or(false)
    }
}

/// 已知设备（持久化存储）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownDevice {
    pub device_id: String, // UUID string for SQLite
    pub device_name: String,
    pub os: String, // OperatingSystem as string
    pub ip_address: String,
    pub quic_port: i64, // i64 for SQLite compatibility
    pub protocol_version: String,
    pub capabilities: String, // JSON array string
    pub cert_fingerprint: String,
    pub last_seen: i64, // Unix timestamp
    pub last_connected: Option<i64>,
    pub is_online: bool,
    pub created_at: i64, // Unix timestamp
}

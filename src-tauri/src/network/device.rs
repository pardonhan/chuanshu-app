use serde::{Serialize, Deserialize};
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
}

impl DeviceInfo {
    pub fn new(
        device_id: Uuid,
        device_name: String,
        os: OperatingSystem,
        ip_address: IpAddr,
        quic_port: u16,
    ) -> Self {
        Self {
            device_id,
            device_name,
            os,
            ip_address,
            quic_port,
            protocol_version: crate::core::constants::PROTOCOL_VERSION.to_string(),
            capabilities: vec![
                Capability::FolderTransfer,
                Capability::ResumeTransfer,
                Capability::MultiDeviceSend,
            ],
            last_seen: SystemTime::now(),
        }
    }

    pub fn is_online(&self) -> bool {
        self.last_seen.elapsed()
            .map(|d| d.as_secs() < crate::core::constants::DISCOVERY_TIMEOUT)
            .unwrap_or(false)
    }
}

/// 已知设备（持久化存储）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownDevice {
    pub device_id: String,       // UUID string for SQLite
    pub device_name: String,
    pub os: String,              // OperatingSystem as string
    pub ip_address: String,
    pub quic_port: i64,          // i64 for SQLite compatibility
    pub protocol_version: String,
    pub capabilities: String,    // JSON array string
    pub last_seen: i64,          // Unix timestamp
    pub last_connected: Option<i64>,
    pub is_online: bool,
    pub created_at: i64,         // Unix timestamp
}

impl KnownDevice {
    /// Convert DeviceInfo to KnownDevice (online)
    pub fn from_device_info(device: &DeviceInfo) -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let capabilities: Vec<String> = device.capabilities.iter().map(|c| {
            match c {
                Capability::FolderTransfer => "FolderTransfer".to_string(),
                Capability::ResumeTransfer => "ResumeTransfer".to_string(),
                Capability::MultiDeviceSend => "MultiDeviceSend".to_string(),
                Capability::P2PTransfer => "P2PTransfer".to_string(),
            }
        }).collect();

        Self {
            device_id: device.device_id.to_string(),
            device_name: device.device_name.clone(),
            os: match &device.os {
                OperatingSystem::Windows => "Windows".to_string(),
                OperatingSystem::MacOS => "MacOS".to_string(),
                OperatingSystem::Linux => "Linux".to_string(),
                OperatingSystem::Unknown => "Unknown".to_string(),
            },
            ip_address: device.ip_address.to_string(),
            quic_port: device.quic_port as i64,
            protocol_version: device.protocol_version.clone(),
            capabilities: serde_json::to_string(&capabilities).unwrap_or_default(),
            last_seen: now,
            last_connected: None,
            is_online: true,
            created_at: now,
        }
    }

    /// Convert to DeviceInfo for use in the app
    pub fn to_device_info(&self) -> Option<DeviceInfo> {
        let device_id = Uuid::parse_str(&self.device_id).ok()?;

        let os = match self.os.as_str() {
            "Windows" => OperatingSystem::Windows,
            "MacOS" => OperatingSystem::MacOS,
            "Linux" => OperatingSystem::Linux,
            _ => OperatingSystem::Unknown,
        };

        let ip_address = self.ip_address.parse().ok()?;

        let capabilities: Vec<String> = serde_json::from_str(&self.capabilities).unwrap_or_default();
        let capabilities = capabilities.iter().map(|c| {
            match c.as_str() {
                "FolderTransfer" => Capability::FolderTransfer,
                "ResumeTransfer" => Capability::ResumeTransfer,
                "MultiDeviceSend" => Capability::MultiDeviceSend,
                "P2PTransfer" => Capability::P2PTransfer,
                _ => Capability::FolderTransfer,
            }
        }).collect();

        let last_seen = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(self.last_seen as u64);

        Some(DeviceInfo {
            device_id,
            device_name: self.device_name.clone(),
            os,
            ip_address,
            quic_port: self.quic_port as u16,
            protocol_version: self.protocol_version.clone(),
            capabilities,
            last_seen,
        })
    }
}

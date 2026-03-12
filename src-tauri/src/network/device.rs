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

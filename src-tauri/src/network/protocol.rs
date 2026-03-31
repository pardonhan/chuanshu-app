use serde::{Serialize, Deserialize};
use std::net::{IpAddr, SocketAddr};
use uuid::Uuid;
use crate::network::device::{DeviceInfo, OperatingSystem, Capability};

/// Protocol version for compatibility checking
pub const PROTOCOL_VERSION: &str = "1.0.0";

/// Maximum UDP packet size
pub const MAX_UDP_PACKET_SIZE: usize = 65507;

/// Discovery message types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiscoveryMessageType {
    /// Broadcast announcement of device presence
    Announce,
    /// Response to an announcement (direct unicast)
    Response,
    /// Goodbye message when device is shutting down
    Goodbye,
}

/// Device discovery packet sent over UDP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryPacket {
    /// Message type
    pub message_type: DiscoveryMessageType,
    /// Protocol version
    pub protocol_version: String,
    /// Device unique ID
    pub device_id: Uuid,
    /// Device display name
    pub device_name: String,
    /// Operating system
    pub os: OperatingSystem,
    /// IP address (can be overridden by sender socket addr)
    pub ip_address: Option<IpAddr>,
    /// QUIC service port
    pub quic_port: u16,
    /// Device capabilities
    pub capabilities: Vec<Capability>,
    /// Timestamp when packet was sent
    pub timestamp: u64,
}

impl DiscoveryPacket {
    /// Create a new announcement packet
    pub fn announce(
        device_id: Uuid,
        device_name: String,
        os: OperatingSystem,
        ip_address: Option<IpAddr>,
        quic_port: u16,
        capabilities: Vec<Capability>,
    ) -> Self {
        Self {
            message_type: DiscoveryMessageType::Announce,
            protocol_version: PROTOCOL_VERSION.to_string(),
            device_id,
            device_name,
            os,
            ip_address,
            quic_port,
            capabilities,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Create a response packet
    pub fn response(&self, local_device: &DeviceInfo) -> Self {
        Self {
            message_type: DiscoveryMessageType::Response,
            protocol_version: PROTOCOL_VERSION.to_string(),
            device_id: local_device.device_id,
            device_name: local_device.device_name.clone(),
            os: local_device.os.clone(),
            ip_address: None, // Will be determined from socket
            quic_port: local_device.quic_port,
            capabilities: local_device.capabilities.clone(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Create a goodbye packet
    pub fn goodbye(device_id: Uuid, device_name: String, os: OperatingSystem, quic_port: u16) -> Self {
        Self {
            message_type: DiscoveryMessageType::Goodbye,
            protocol_version: PROTOCOL_VERSION.to_string(),
            device_id,
            device_name,
            os,
            ip_address: None,
            quic_port,
            capabilities: vec![],
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    /// Serialize to bytes using bincode
    pub fn to_bytes(&self) -> crate::core::AppResult<Vec<u8>> {
        Ok(bincode::serialize(self)?)
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> crate::core::AppResult<Self> {
        Ok(bincode::deserialize(data)?)
    }

    /// Convert to DeviceInfo using socket address for IP
    pub fn to_device_info(&self, socket_addr: SocketAddr) -> DeviceInfo {
        let ip = self.ip_address.unwrap_or(socket_addr.ip());
        DeviceInfo {
            device_id: self.device_id,
            device_name: self.device_name.clone(),
            os: self.os.clone(),
            ip_address: ip,
            quic_port: self.quic_port,
            protocol_version: self.protocol_version.clone(),
            capabilities: self.capabilities.clone(),
            last_seen: std::time::SystemTime::now(),
        }
    }
}

/// Control message types for QUIC connections
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ControlMessageType {
    /// Authentication handshake - Hello
    AuthHello,
    /// Authentication handshake - Response with certificate fingerprint
    AuthResponse,
    /// Authentication handshake - Acknowledge
    AuthAck,
    /// File transfer request
    TransferRequest,
    /// File transfer response (accept/reject)
    TransferResponse,
    /// Cancel ongoing transfer
    CancelTransfer,
    /// Pause transfer
    PauseTransfer,
    /// Resume transfer
    ResumeTransfer,
    /// Query transfer progress
    QueryProgress,
    /// Progress update
    ProgressUpdate,
    /// Error notification
    Error,
}

/// Authentication hello message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthHello {
    /// Device ID
    pub device_id: Uuid,
    /// Device name
    pub device_name: String,
    /// Protocol version
    pub protocol_version: String,
    /// Random nonce for challenge-response
    pub nonce: Vec<u8>,
}

/// Authentication response message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    /// Device ID
    pub device_id: Uuid,
    /// Device name
    pub device_name: String,
    /// Certificate fingerprint (SHA-256)
    pub cert_fingerprint: String,
    /// Signature of the nonce from AuthHello
    pub signature: Vec<u8>,
    /// Random nonce for challenge-response
    pub nonce: Vec<u8>,
}

/// Authentication acknowledge message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthAck {
    /// Device ID
    pub device_id: Uuid,
    /// Signature of the nonce from AuthResponse
    pub signature: Vec<u8>,
    /// Authentication result
    pub authenticated: bool,
    /// Error message if authentication failed
    pub error: Option<String>,
}

/// File metadata for transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    /// Unique file ID within transfer
    pub file_id: u64,
    /// Relative path (for folder transfers)
    pub relative_path: String,
    /// File name
    pub file_name: String,
    /// File size in bytes
    pub file_size: u64,
    /// File modification time (Unix timestamp)
    pub modified_time: u64,
    /// File checksum (SHA-256)
    pub checksum: Option<String>,
    /// Is directory
    pub is_directory: bool,
    /// Source file path (for sender, not transmitted)
    #[serde(skip)]
    pub source_path: String,
}

/// Transfer request message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRequest {
    /// Transfer task ID
    pub task_id: Uuid,
    /// Sender device ID
    pub sender_id: Uuid,
    /// Sender device name
    pub sender_name: String,
    /// Total number of files
    pub file_count: u32,
    /// Total size in bytes
    pub total_size: u64,
    /// Files to transfer
    pub files: Vec<FileMetadata>,
    /// Resume from existing transfer
    pub resume: bool,
}

/// Transfer response message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferResponse {
    /// Transfer task ID
    pub task_id: Uuid,
    /// Accepted or rejected
    pub accepted: bool,
    /// Rejection reason (if rejected)
    pub reason: Option<String>,
    /// Suggested save path
    pub save_path: Option<String>,
}

/// Chunk metadata for data transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMetadata {
    /// Transfer task ID
    pub task_id: Uuid,
    /// File ID
    pub file_id: u64,
    /// Chunk index (0-based)
    pub chunk_index: u64,
    /// Total chunks for this file
    pub total_chunks: u64,
    /// Chunk size
    pub chunk_size: u32,
    /// Chunk offset in file
    pub offset: u64,
    /// Chunk checksum (CRC32)
    pub checksum: u32,
}

/// Control message envelope
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlMessage {
    /// Message type
    pub message_type: ControlMessageType,
    /// Payload (type-specific serialized data)
    pub payload: Vec<u8>,
}

impl ControlMessage {
    /// Create a new control message
    pub fn new<T: Serialize>(message_type: ControlMessageType, payload: &T) -> crate::core::AppResult<Self> {
        Ok(Self {
            message_type,
            payload: bincode::serialize(payload)?,
        })
    }

    /// Deserialize payload
    pub fn payload<T: for<'de> Deserialize<'de>>(&self) -> crate::core::AppResult<T> {
        Ok(bincode::deserialize(&self.payload)?)
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> crate::core::AppResult<Vec<u8>> {
        Ok(bincode::serialize(self)?)
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> crate::core::AppResult<Self> {
        Ok(bincode::deserialize(data)?)
    }
}

/// Data packet for file chunks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPacket {
    /// Chunk metadata
    pub metadata: ChunkMetadata,
    /// Actual data (may be empty for metadata-only packets)
    pub data: Vec<u8>,
}

impl DataPacket {
    /// Serialize to bytes
    pub fn to_bytes(&self) -> crate::core::AppResult<Vec<u8>> {
        Ok(bincode::serialize(self)?)
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> crate::core::AppResult<Self> {
        Ok(bincode::deserialize(data)?)
    }
}

/// Transfer progress update
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressUpdate {
    /// Transfer task ID
    pub task_id: Uuid,
    /// Current file ID
    pub current_file_id: u64,
    /// Current file name
    pub current_file_name: String,
    /// Bytes transferred for current file
    pub file_transferred: u64,
    /// Total bytes transferred
    pub total_transferred: u64,
    /// Current speed (bytes per second)
    pub speed: u64,
    /// Is completed
    pub completed: bool,
    /// Error message (if failed)
    pub error: Option<String>,
}

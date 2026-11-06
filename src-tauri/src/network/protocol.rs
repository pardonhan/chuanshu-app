use serde::{Serialize, Deserialize};
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

use crate::core::{AppError, AppResult, TRANSFER_PROTOCOL_VERSION};
use crate::network::device::{now_secs, DeviceInfo, OperatingSystem};

/// UDP 发现协议版本（字符串）
pub const PROTOCOL_VERSION: &str = crate::core::constants::PROTOCOL_VERSION;

/// 控制帧（请求/响应/文件头）最大长度：8MB
pub const MAX_CONTROL_FRAME_SIZE: usize = 8 * 1024 * 1024;

// ==================== 传输协议 v2 帧标签 ====================
//
// 一条双向流内的顺序：
//   u32 len + bincode(TransferRequest)          <- 发送端
//   u32 len + bincode(TransferResponse)         <- 接收端
//   逐个文件：
//     0x03 + u32 len + bincode(FileHeader)
//     0x01 + u64 offset + u32 data_len + u32 crc32 + bytes
//     0x02 + u8 has_sha256 + [32] sha256
//   0x00                                        <- 全部结束
//   u8 0x00 (OK) 或 0x01 + u32 len + utf8 错误信息  <- 接收端确认
pub const TAG_TRANSFER_END: u8 = 0x00;
pub const TAG_DATA: u8 = 0x01;
pub const TAG_FILE_END: u8 = 0x02;
pub const TAG_FILE_HEADER: u8 = 0x03;

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
    pub message_type: DiscoveryMessageType,
    pub protocol_version: String,
    pub device_id: Uuid,
    pub device_name: String,
    pub os: OperatingSystem,
    pub ip_address: Option<std::net::IpAddr>,
    pub quic_port: u16,
    pub capabilities: Vec<crate::network::device::Capability>,
    pub timestamp: u64,
}

impl DiscoveryPacket {
    pub fn announce(
        device_id: Uuid,
        device_name: String,
        os: OperatingSystem,
        ip_address: Option<std::net::IpAddr>,
        quic_port: u16,
        capabilities: Vec<crate::network::device::Capability>,
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
            timestamp: now_secs(),
        }
    }

    pub fn response(&self, local_device: &DeviceInfo) -> Self {
        Self {
            message_type: DiscoveryMessageType::Response,
            protocol_version: PROTOCOL_VERSION.to_string(),
            device_id: local_device.device_id,
            device_name: local_device.device_name.clone(),
            os: local_device.os.clone(),
            ip_address: None,
            quic_port: local_device.quic_port,
            capabilities: local_device.capabilities.clone(),
            timestamp: now_secs(),
        }
    }

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
            timestamp: now_secs(),
        }
    }

    pub fn to_bytes(&self) -> AppResult<Vec<u8>> {
        Ok(bincode::serialize(self)?)
    }

    pub fn from_bytes(data: &[u8]) -> AppResult<Self> {
        Ok(bincode::deserialize(data)?)
    }

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
            last_seen: now_secs(),
        }
    }
}

// ==================== 传输协议 v2 ====================

/// 文件元数据（请求中携带）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    pub file_id: u64,
    /// 相对目录（可为空），用于保留目录结构
    pub relative_path: String,
    pub file_name: String,
    pub file_size: u64,
}

/// 传输请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRequest {
    pub protocol_version: u16,
    pub task_id: Uuid,
    pub sender_id: Uuid,
    pub sender_name: String,
    pub total_size: u64,
    pub files: Vec<FileMeta>,
}

impl TransferRequest {
    pub fn new(
        task_id: Uuid,
        sender_id: Uuid,
        sender_name: String,
        total_size: u64,
        files: Vec<FileMeta>,
    ) -> Self {
        Self {
            protocol_version: TRANSFER_PROTOCOL_VERSION,
            task_id,
            sender_id,
            sender_name,
            total_size,
            files,
        }
    }
}

/// 传输响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferResponse {
    pub task_id: Uuid,
    pub accepted: bool,
    pub reason: Option<String>,
    pub save_path: Option<String>,
}

/// 每个文件开始前的文件头
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHeader {
    pub file_id: u64,
    pub file_name: String,
    pub relative_path: String,
    pub file_size: u64,
}

// ==================== 帧读写辅助 ====================

/// 写一个长度前缀帧（u32 大端 + payload）
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) -> AppResult<()> {
    if payload.len() > MAX_CONTROL_FRAME_SIZE {
        return Err(AppError::Other("控制帧过大".to_string()));
    }
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .map_err(|e| AppError::Network(format!("写入长度失败: {}", e)))?;
    writer
        .write_all(payload)
        .await
        .map_err(|e| AppError::Network(format!("写入数据失败: {}", e)))?;
    Ok(())
}

/// 读一个长度前缀帧，超长直接报错
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R, max_len: usize) -> AppResult<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| AppError::Network(format!("读取长度失败: {}", e)))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > max_len {
        return Err(AppError::Other(format!("帧过大: {} 字节", len)));
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| AppError::Network(format!("读取数据失败: {}", e)))?;
    Ok(buf)
}

/// 写序列化帧
pub async fn write_serialized<W, T>(writer: &mut W, value: &T) -> AppResult<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let data = bincode::serialize(value)?;
    write_frame(writer, &data).await
}

/// 读并反序列化帧
pub async fn read_serialized<R, T>(reader: &mut R, max_len: usize) -> AppResult<T>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let data = read_frame(reader, max_len).await?;
    Ok(bincode::deserialize(&data)?)
}

/// 设备发现UDP端口
pub const DISCOVERY_PORT: u16 = 45678;
/// 设备发现心跳间隔（秒）
pub const DISCOVERY_HEARTBEAT_INTERVAL: u64 = 3;
/// 设备离线超时时间（秒） - 超过此时间未收到心跳视为离线
pub const DISCOVERY_TIMEOUT: u64 = 30;
/// QUIC 连接空闲超时时间（秒）
pub const QUIC_IDLE_TIMEOUT: u64 = 60;
/// QUIC 保活间隔（秒）
pub const QUIC_KEEP_ALIVE_INTERVAL: u64 = 10;
/// QUIC服务端默认端口
pub const QUIC_DEFAULT_PORT: u16 = 45679;
/// 默认块大小（4MB）
pub const DEFAULT_CHUNK_SIZE: u32 = 4 * 1024 * 1024;
/// 传输协议版本
pub const TRANSFER_PROTOCOL_VERSION: u16 = 2;
/// 发现报文协议版本（字符串，仅用于 UDP 发现）
pub const PROTOCOL_VERSION: &str = "1.0.0";

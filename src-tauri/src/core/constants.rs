/// 设备发现UDP端口
pub const DISCOVERY_PORT: u16 = 45678;
/// 设备发现心跳间隔（秒）
pub const DISCOVERY_HEARTBEAT_INTERVAL: u64 = 3;
/// 设备离线超时时间（秒） - 超过此时间未收到心跳视为离线
pub const DISCOVERY_TIMEOUT: u64 = 30;
/// 设备重新上线判定窗口（秒） - 超过此时间未见视为“曾离线”
pub const DISCOVERY_REJOIN_TIMEOUT: u64 = 25;
/// QUIC 连接空闲超时时间（秒）- 必须大于 DISCOVERY_TIMEOUT
pub const QUIC_IDLE_TIMEOUT: u64 = 60;
/// QUIC 保活间隔（秒）- 应小于空闲超时的 1/3
pub const QUIC_KEEP_ALIVE_INTERVAL: u64 = 15;
/// QUIC服务端默认端口
pub const QUIC_DEFAULT_PORT: u16 = 45679;
/// 最大并发发送任务数
pub const MAX_CONCURRENT_TRANSFERS: usize = 5;
/// 任务终态后保留在内存中的时间（秒）
pub const TRANSFER_TASK_RETENTION: u64 = 60 * 60;
/// 小文件阈值（256KB）
pub const SMALL_FILE_THRESHOLD: u64 = 256 * 1024;
/// 协议版本（1.1.0 起发现包携带证书指纹）
pub const PROTOCOL_VERSION: &str = "1.1.0";

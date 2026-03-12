/// 设备发现UDP端口
pub const DISCOVERY_PORT: u16 = 45678;
/// 设备发现心跳间隔（秒）
pub const DISCOVERY_HEARTBEAT_INTERVAL: u64 = 3;
/// 设备离线超时时间（秒）
pub const DISCOVERY_TIMEOUT: u64 = 9;
/// QUIC服务端默认端口
pub const QUIC_DEFAULT_PORT: u16 = 45679;
/// 最大并发传输任务数
pub const MAX_CONCURRENT_TRANSFERS: usize = 5;
/// 默认块大小（4MB）
pub const DEFAULT_CHUNK_SIZE: u32 = 4 * 1024 * 1024;
/// 小文件阈值（256KB），小于此大小的文件将批量传输
pub const SMALL_FILE_THRESHOLD: u64 = 256 * 1024;
/// 批量传输最大文件数
pub const MAX_BATCH_FILES: usize = 100;
/// 批量传输最大总大小（64MB）
pub const MAX_BATCH_SIZE: u64 = 64 * 1024 * 1024;
/// 协议版本
pub const PROTOCOL_VERSION: &str = "1.0.0";

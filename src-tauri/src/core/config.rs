use serde::{Deserialize, Serialize};

/// 应用设置。
///
/// `upload_limit` / `download_limit` 的单位是 **MB/s**，0 表示不限速。
/// 传输层使用 [`Settings::upload_bytes_per_sec`] / [`Settings::download_bytes_per_sec`]
/// 转换为字节/秒。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Settings {
    pub device_name: String,
    pub download_path: String,
    pub auto_accept: bool,
    pub upload_limit: u32,
    pub download_limit: u32,
    pub enable_notification: bool,
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: "我的设备".to_string(),
            download_path: "~/Downloads/传书".to_string(),
            auto_accept: false,
            upload_limit: 0,
            download_limit: 0,
            enable_notification: true,
            theme: "auto".to_string(),
        }
    }
}

impl Settings {
    pub fn upload_bytes_per_sec(&self) -> u32 {
        mb_per_sec_to_bytes(self.upload_limit)
    }

    pub fn download_bytes_per_sec(&self) -> u32 {
        mb_per_sec_to_bytes(self.download_limit)
    }
}

fn mb_per_sec_to_bytes(limit_mb: u32) -> u32 {
    limit_mb.saturating_mul(1024 * 1024)
}

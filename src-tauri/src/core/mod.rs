pub mod error;
pub mod constants;
pub mod app_state;

pub use error::*;
pub use constants::*;
pub use app_state::*;

use std::path::PathBuf;

/// 用户主目录（Windows 用 USERPROFILE，其他系统用 HOME）
pub fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 默认下载目录：<home>/Downloads/传书
pub fn default_download_dir() -> PathBuf {
    home_dir().join("Downloads").join("传书")
}

/// 解析设置中的下载目录（展开 `~` 前缀）
pub fn resolve_download_dir(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return default_download_dir();
    }

    let expanded = if let Some(rest) = trimmed.strip_prefix("~/").or_else(|| trimmed.strip_prefix("~\\")) {
        home_dir().join(rest)
    } else if trimmed == "~" {
        home_dir()
    } else {
        PathBuf::from(trimmed)
    };

    expanded
}

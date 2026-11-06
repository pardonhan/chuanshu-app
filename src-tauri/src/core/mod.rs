pub mod error;
pub mod constants;
pub mod app_state;

pub use error::*;
pub use constants::*;
pub use app_state::*;

use std::path::{Path, PathBuf};

use tauri::Manager;

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

/// 接收目录不可写时的兜底目录：应用数据目录下的 Downloads
pub fn app_data_download_dir(state: &AppState) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CHUANSHU_DATA_DIR") {
        return Some(PathBuf::from(dir).join("Downloads"));
    }

    state
        .get_app_handle()
        .and_then(|handle| handle.path().app_data_dir().ok())
        .map(|dir| dir.join("Downloads"))
}

/// 校验目录是否可写：必要时创建目录，并写入探针文件验证真实写权限。
/// macOS 上首次访问「下载」文件夹时，这一步会触发系统授权弹窗。
pub fn ensure_dir_writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;

    let probe = dir.join(format!(".chuanshu_write_probe_{}", std::process::id()));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&probe)?;
        file.write_all(b"chuanshu")?;
        file.sync_all()?;
    }
    // 探针文件清理失败不影响可写判定
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// 按顺序返回第一个可写的候选目录
pub fn first_writable_dir(candidates: &[PathBuf]) -> Option<PathBuf> {
    for dir in candidates {
        match ensure_dir_writable(dir) {
            Ok(()) => return Some(dir.clone()),
            Err(e) => log::warn!("目录不可写 {:?}: {}", dir, e),
        }
    }
    None
}

/// 写权限失败时给用户的平台化处理建议
pub fn permission_hint() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "请在「系统设置 → 隐私与安全性 → 文件与文件夹」中允许本应用访问该目录，或重新选择一个有写权限的目录"
    }
    #[cfg(target_os = "windows")]
    {
        "请确认目录存在且未被占用，或选择一个具有写权限的目录"
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        "请检查目录权限（chmod）或选择一个有写权限的目录"
    }
}

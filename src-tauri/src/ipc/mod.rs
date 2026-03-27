use serde::{Serialize, Deserialize};
use std::sync::Arc;
use tauri::{Manager, State, Emitter};
use tokio::runtime::Handle;
use uuid::Uuid;
use crate::core::*;
use crate::network::DeviceInfo;
use crate::transfer::TransferTaskInfo;
use crate::storage::{get_storage, TransferHistoryEntry, KnownDevice};

/// 系统信息
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemInfo {
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub default_download_path: String,
}

/// 获取系统信息
#[tauri::command]
pub async fn get_system_info() -> AppResult<SystemInfo> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let os_version = match os {
        "windows" => {
            if let Some(version) = windows_version() {
                version
            } else {
                "Unknown".to_string()
            }
        }
        "macos" => {
            // macOS 版本可以通过 sysctl 获取，这里简化处理
            "macOS".to_string()
        }
        "linux" => {
            // 尝试读取 /etc/os-release
            if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
                for line in content.lines() {
                    if line.starts_with("PRETTY_NAME=") {
                        return Ok(SystemInfo {
                            os: os.to_string(),
                            os_version: line.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string(),
                            arch: arch.to_string(),
                            default_download_path: get_default_download_path(),
                        });
                    }
                }
            }
            "Linux".to_string()
        }
        _ => "Unknown".to_string(),
    };

    Ok(SystemInfo {
        os: os.to_string(),
        os_version,
        arch: arch.to_string(),
        default_download_path: get_default_download_path(),
    })
}

/// 获取 Windows 版本
#[cfg(target_os = "windows")]
fn windows_version() -> Option<String> {
    // 使用简单的注册表查询
    use std::process::Command;
    Command::new("powershell")
        .args(["-Command", "(Get-ItemProperty -Path \"Registry::HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\" -Name ProductName).ProductName"])
        .output()
        .ok()
        .and_then(|output| {
            String::from_utf8(output.stdout).ok()
        })
        .map(|s| s.trim().to_string())
}

#[cfg(not(target_os = "windows"))]
fn windows_version() -> Option<String> {
    None
}

/// 获取默认下载路径
fn get_default_download_path() -> String {
    #[cfg(target_os = "windows")]
    {
        // Windows: C:\Users\{user}\Downloads
        if let Ok(home) = std::env::var("USERPROFILE") {
            return format!("{}\\Downloads\\传书", home);
        }
    }

    #[cfg(target_os = "macos")]
    {
        // macOS: /Users/{user}/Downloads
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/Downloads/传书", home);
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: /home/{user}/Downloads
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/Downloads/传书", home);
        }
    }

    "~/Downloads/传书".to_string()
}

/// 设置开机自启
#[tauri::command]
pub async fn set_auto_launch(enabled: bool, app_handle: tauri::AppHandle) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let current_exe = std::env::current_exe()?;
        let exe_path = current_exe.to_string_lossy();

        if enabled {
            // 添加到注册表
            let reg_path = "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
            Command::new("reg")
                .args(["add", reg_path, "/v", "ChuanshuApp", "/t", "REG_SZ", "/d", &exe_path, "/f"])
                .output()?;
        } else {
            // 从注册表删除
            Command::new("reg")
                .args(["delete", "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "ChuanshuApp", "/f"])
                .output()?;
        }
    }

    #[cfg(target_os = "macos")]
    {
        let current_exe = std::env::current_exe()?;
        let exe_path = current_exe.to_string_lossy();
        let bundle_id = "com.chuanshu.app";

        if enabled {
            // 创建 LaunchAgent plist
            let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
            let plist_path = format!("{}/Library/LaunchAgents/{}.plist", home, bundle_id);
            let plist_content = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>"#, bundle_id, exe_path);
            std::fs::write(&plist_path, plist_content)?;
        } else {
            // 删除 LaunchAgent plist
            let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
            let plist_path = format!("{}/Library/LaunchAgents/{}.plist", home, bundle_id);
            let _ = std::fs::remove_file(&plist_path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: 创建.desktop 文件
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        let config_dir = format!("{}/.config/autostart", home);
        let desktop_path = format!("{}/chuanshu.desktop", config_dir);

        if enabled {
            std::fs::create_dir_all(&config_dir)?;
            let current_exe = std::env::current_exe()?;
            let exe_path = current_exe.to_string_lossy();
            let desktop_content = format!(r#"[Desktop Entry]
Type=Application
Name=传书
Exec={}
Comment=局域网文件传输工具
Terminal=false
"#, exe_path);
            std::fs::write(&desktop_path, desktop_content)?;
        } else {
            let _ = std::fs::remove_file(&desktop_path);
        }
    }

    let _ = app_handle; // 避免未使用警告
    Ok(())
}

/// 获取开机自启状态
#[tauri::command]
pub async fn get_auto_launch() -> AppResult<bool> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let output = Command::new("reg")
            .args(["query", "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "ChuanshuApp"])
            .output();
        return Ok(output.map_or(false, |o| o.status.success()));
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        let bundle_id = "com.chuanshu.app";
        let plist_path = format!("{}/Library/LaunchAgents/{}.plist", home, bundle_id);
        return Ok(std::path::Path::new(&plist_path).exists());
    }

    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        let desktop_path = format!("{}/.config/autostart/chuanshu.desktop", home);
        return Ok(std::path::Path::new(&desktop_path).exists());
    }
}

#[tauri::command]
pub async fn get_device_list(state: State<'_, Arc<AppState>>) -> AppResult<Vec<DeviceInfo>> {
    let devices = state.devices
        .iter()
        .map(|entry| entry.value().clone())
        .filter(|d| d.is_online())
        .collect();
    Ok(devices)
}

#[tauri::command]
pub async fn get_transfer_tasks(state: State<'_, Arc<AppState>>) -> AppResult<Vec<TransferTaskInfo>> {
    let mut tasks = Vec::new();
    for entry in state.transfer_tasks.iter() {
        let task = entry.value().lock().await;
        tasks.push(TransferTaskInfo::from(&*task));
    }
    // Sort by created_at descending
    tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(tasks)
}

#[derive(Debug, Deserialize)]
pub struct SendFilesRequest {
    pub device_ids: Vec<Uuid>,
    pub file_paths: Vec<String>,
}

#[tauri::command]
pub async fn send_files(
    request: SendFilesRequest,
    state: State<'_, Arc<AppState>>,
    rt_handle: State<'_, Handle>,
) -> AppResult<Vec<Uuid>> {
    let task_ids = crate::transfer::send_files(request, state.inner().clone(), rt_handle.inner().clone()).await?;
    Ok(task_ids)
}

#[tauri::command]
pub async fn cancel_transfer(task_id: Uuid, state: State<'_, Arc<AppState>>) -> AppResult<()> {
    crate::transfer::cancel_transfer(task_id, state.inner().clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn pause_transfer(task_id: Uuid, state: State<'_, Arc<AppState>>) -> AppResult<()> {
    crate::transfer::pause_transfer(task_id, state.inner().clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn resume_transfer(task_id: Uuid, state: State<'_, Arc<AppState>>) -> AppResult<()> {
    crate::transfer::resume_transfer(task_id, state.inner().clone()).await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Settings {
    pub device_name: String,
    pub download_path: String,
    pub auto_accept: bool,
    pub upload_limit: u32,
    pub download_limit: u32,
    pub enable_notification: bool,
    pub theme: String,
}

#[tauri::command]
pub async fn get_settings() -> AppResult<Settings> {
    if let Some(storage) = get_storage() {
        if let Some(settings) = storage.load_settings()? {
            return Ok(settings);
        }
    }

    // Return default settings
    Ok(Settings {
        device_name: "我的设备".to_string(),
        download_path: "~/Downloads/传书".to_string(),
        auto_accept: false,
        upload_limit: 0,
        download_limit: 0,
        enable_notification: true,
        theme: "auto".to_string(),
    })
}

#[tauri::command]
pub async fn save_settings(settings: Settings) -> AppResult<()> {
    if let Some(storage) = get_storage() {
        storage.save_settings(&settings)?;
    }
    log::info!("Settings saved: device_name={}", settings.device_name);
    Ok(())
}

/// Get transfer history with pagination
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct TransferHistoryQuery {
    pub limit: i64,
    pub offset: i64,
}

#[tauri::command]
pub async fn get_transfer_history(query: TransferHistoryQuery) -> AppResult<Vec<TransferHistoryEntry>> {
    let storage = get_storage().ok_or_else(|| crate::core::AppError::Other("Storage not initialized".to_string()))?;
    let history = storage.get_transfer_history(query.limit, query.offset)?;
    Ok(history)
}

/// Clear transfer history
#[tauri::command]
pub async fn clear_transfer_history() -> AppResult<()> {
    let storage = get_storage().ok_or_else(|| crate::core::AppError::Other("Storage not initialized".to_string()))?;
    storage.clear_transfer_history()?;
    Ok(())
}

/// Get known devices (including offline devices)
#[tauri::command]
pub async fn get_known_devices() -> AppResult<Vec<KnownDevice>> {
    let storage = get_storage().ok_or_else(|| crate::core::AppError::Other("Storage not initialized".to_string()))?;
    let devices = storage.get_known_devices()?;
    Ok(devices)
}

/// Delete a known device
#[tauri::command]
pub async fn delete_known_device(device_id: String) -> AppResult<()> {
    let uuid = Uuid::parse_str(&device_id)
        .map_err(|e| crate::core::AppError::Other(format!("Invalid device ID: {}", e)))?;
    let storage = get_storage().ok_or_else(|| crate::core::AppError::Other("Storage not initialized".to_string()))?;
    storage.delete_known_device(uuid)?;
    Ok(())
}

/// Open file location in file manager
#[tauri::command]
pub async fn open_file_location(file_path: String, _app_handle: tauri::AppHandle) -> AppResult<()> {
    log::info!("Opening file location: {}", file_path);

    // Get the parent directory
    let path = std::path::Path::new(&file_path);
    let parent = path.parent()
        .ok_or_else(|| crate::core::AppError::Other("Invalid file path".to_string()))?;

    // Open the parent directory using OS-specific command
    let parent_str = parent.to_string_lossy().to_string();

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(&parent_str).spawn();
    }

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(&parent_str).spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(&parent_str).spawn();
    }

    Ok(())
}

/// Resend files to a device (from history)
#[tauri::command]
pub async fn resend_files(
    device_ids: Vec<String>,
    file_paths: Vec<String>,
    state: State<'_, Arc<AppState>>,
    rt_handle: State<'_, Handle>,
) -> AppResult<Vec<String>> {
    use crate::network::protocol::FileMetadata;

    log::info!("Resending files to devices {:?}: {:?}", device_ids, file_paths);

    // Parse device IDs
    let device_uuids = device_ids.iter()
        .map(|id| Uuid::parse_str(id)
            .map_err(|e| crate::core::AppError::Other(format!("Invalid device ID {}: {}", id, e))))
        .collect::<AppResult<Vec<_>>>()?;

    // Create file metadata for each file
    let mut files = Vec::new();

    for file_path in &file_paths {
        let metadata = std::fs::metadata(file_path)
            .map_err(|e| crate::core::AppError::Other(format!("Cannot read file {}: {}", file_path, e)))?;

        let file_name = std::path::Path::new(file_path)
            .file_name()
            .ok_or_else(|| crate::core::AppError::Other("Invalid file path".to_string()))?
            .to_string_lossy()
            .to_string();

        let file_id = files.len() as u64;

        files.push(FileMetadata {
            file_id,
            relative_path: String::new(),
            file_name: file_name.clone(),
            file_size: metadata.len(),
            modified_time: metadata.modified()
                .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
                .unwrap_or(0),
            checksum: None,
            is_directory: false,
            source_path: file_path.clone(),
        });
    }

    // Send files using the existing send mechanism
    let task_ids = crate::transfer::send_files(
        SendFilesRequest {
            device_ids: device_uuids,
            file_paths: file_paths.clone(),
        },
        state.inner().clone(),
        rt_handle.inner().clone(),
    ).await?;

    Ok(task_ids.iter().map(|u| u.to_string()).collect())
}

/// Event names for frontend communication
pub const DEVICE_ONLINE_EVENT: &str = "device-online";
pub const DEVICE_OFFLINE_EVENT: &str = "device-offline";
pub const TRANSFER_PROGRESS_EVENT: &str = "transfer-progress";
pub const TRANSFER_COMPLETED_EVENT: &str = "transfer-completed";
pub const TRANSFER_FAILED_EVENT: &str = "transfer-failed";

/// Send system notification
pub fn send_notification(app_handle: &tauri::AppHandle, title: &str, body: &str) {
    // Check if notifications are enabled in settings
    if let Some(storage) = crate::storage::get_storage() {
        if let Ok(Some(settings)) = storage.load_settings() {
            if !settings.enable_notification {
                log::debug!("Notification disabled: {} - {}", title, body);
                return;
            }
        }
    }

    use tauri_plugin_notification::NotificationExt;

    log::info!("Sending notification: {} - {}", title, body);

    let _ = app_handle.notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

/// Emit device online event
pub fn emit_device_online(handle: &tauri::AppHandle, device: &DeviceInfo) {
    let _ = handle.emit(DEVICE_ONLINE_EVENT, device);
}

/// Emit device offline event
pub fn emit_device_offline(handle: &tauri::AppHandle, device_id: Uuid) {
    let _ = handle.emit(DEVICE_OFFLINE_EVENT, device_id);
}

/// Emit transfer progress event
pub fn emit_transfer_progress(handle: &tauri::AppHandle, task: &TransferTaskInfo) {
    let _ = handle.emit(TRANSFER_PROGRESS_EVENT, task);
}

/// Emit transfer completed event and send notification
pub fn emit_transfer_completed(handle: &tauri::AppHandle, task_id: Uuid, device_name: &str) {
    let _ = handle.emit(TRANSFER_COMPLETED_EVENT, task_id);

    // Send system notification
    send_notification(handle, "传输完成", &format!("文件已从 {} 接收完成", device_name));
}

/// Emit transfer failed event and send notification
pub fn emit_transfer_failed(handle: &tauri::AppHandle, task_id: Uuid, error: String) {
    let _ = handle.emit(TRANSFER_FAILED_EVENT, serde_json::json!({
        "task_id": task_id,
        "error": error
    }));

    // Send system notification
    send_notification(handle, "传输失败", &format!("文件传输失败：{}", error));
}

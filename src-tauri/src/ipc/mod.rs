use serde::{Serialize, Deserialize};
use std::sync::Arc;
use tauri::{State, Emitter};
use uuid::Uuid;
use crate::core::*;
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

/// 下载目录权限检测结果
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadPathStatus {
    /// 实际检测的目录（已展开 `~`）
    pub path: String,
    /// 是否可写
    pub writable: bool,
    /// 不可写时的原因与处理建议
    pub message: Option<String>,
    /// 可自动切换到的兜底目录
    pub suggested_path: Option<String>,
}

/// 检测并尝试自动获取下载目录的写权限。
/// macOS 下首次访问「下载」目录会触发系统授权弹窗；配置目录不可写时返回可用的兜底目录。
#[tauri::command]
pub async fn check_download_path(
    path: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> AppResult<DownloadPathStatus> {
    let configured_dir = path
        .as_deref()
        .map(resolve_download_dir)
        .unwrap_or_else(default_download_dir);

    if let Err(e) = ensure_dir_writable(&configured_dir) {
        log::warn!("下载目录不可写 {:?}: {}", configured_dir, e);

        let mut candidates = vec![default_download_dir()];
        if let Some(fallback) = app_data_download_dir(state.inner()) {
            if !candidates.contains(&fallback) {
                candidates.push(fallback);
            }
        }
        let suggested = first_writable_dir(&candidates);

        return Ok(DownloadPathStatus {
            path: configured_dir.to_string_lossy().to_string(),
            writable: false,
            message: Some(format!(
                "无法写入 {:?}：{}。{}",
                configured_dir,
                e,
                permission_hint()
            )),
            suggested_path: suggested.map(|p| p.to_string_lossy().to_string()),
        });
    }

    Ok(DownloadPathStatus {
        path: configured_dir.to_string_lossy().to_string(),
        writable: true,
        message: None,
        suggested_path: None,
    })
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
    crate::core::default_download_dir()
        .to_string_lossy()
        .to_string()
}

/// 设置开机自启
#[tauri::command]
pub async fn set_auto_launch(enabled: bool, app_handle: tauri::AppHandle) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let current_exe = std::env::current_exe()?;
        let exe_path = current_exe.to_string_lossy();

        let reg_path = "HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
        let output = if enabled {
            // 添加到注册表
            Command::new("reg")
                .args(["add", reg_path, "/v", "ChuanshuApp", "/t", "REG_SZ", "/d", &exe_path, "/f"])
                .output()?
        } else {
            // 从注册表删除
            Command::new("reg")
                .args(["delete", reg_path, "/v", "ChuanshuApp", "/f"])
                .output()?
        };

        if !output.status.success() {
            return Err(AppError::Other(format!(
                "设置开机自启失败: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LocalDeviceInfo {
    pub device_id: String,
    pub device_name: String,
}

/// 获取本机设备信息（设备 ID 持久化）
#[tauri::command]
pub async fn get_local_device_info(state: State<'_, Arc<AppState>>) -> AppResult<LocalDeviceInfo> {
    Ok(LocalDeviceInfo {
        device_id: state.device_id.to_string(),
        device_name: state.device_name(),
    })
}

#[tauri::command]
pub async fn get_transfer_tasks(state: State<'_, Arc<AppState>>) -> AppResult<Vec<TransferTaskInfo>> {
    let arcs: Vec<Arc<tokio::sync::Mutex<crate::transfer::TransferTask>>> = state
        .transfer_tasks
        .iter()
        .map(|entry| entry.value().clone())
        .collect();

    let mut tasks = Vec::with_capacity(arcs.len());
    for arc in arcs {
        let task = arc.lock().await;
        tasks.push(TransferTaskInfo::from(&*task));
    }
    // Sort by created_at descending
    tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(tasks)
}

/// 取出指定任务中登记的第 index 个本地文件路径
async fn resolve_task_file(
    task_id: Uuid,
    index: usize,
    state: &Arc<AppState>,
) -> AppResult<std::path::PathBuf> {
    let arc = state
        .transfer_tasks
        .get(&task_id)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| AppError::DeviceNotFound("传输任务不存在".to_string()))?;

    let task = arc.lock().await;
    if task.status != crate::transfer::TransferStatus::Completed {
        return Err(AppError::Other("只有已完成的传输才能打开文件".to_string()));
    }

    let path = task
        .file_paths
        .get(index)
        .ok_or_else(|| AppError::Other("文件路径不存在".to_string()))?;
    Ok(std::path::PathBuf::from(path))
}

/// 调用系统默认程序打开文件（reveal=true 时在文件管理器中定位该文件）
fn open_in_system(path: &std::path::Path, reveal: bool) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    let result = if reveal {
        std::process::Command::new("open").arg("-R").arg(path).spawn()
    } else {
        std::process::Command::new("open").arg(path).spawn()
    };

    #[cfg(target_os = "windows")]
    let result = if reveal {
        std::process::Command::new("explorer")
            .arg(format!("/select,\"{}\"", path.display()))
            .spawn()
    } else {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg("start")
            .arg("")
            .arg(path)
            .spawn()
    };

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let result = if reveal {
        std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or(path))
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };

    result
        .map(|_| ())
        .map_err(|e| AppError::Other(format!("无法打开文件: {}", e)))
}

/// 用系统默认程序打开已传输成功的文件
#[tauri::command]
pub async fn open_transfer_file(
    task_id: Uuid,
    index: usize,
    state: State<'_, Arc<AppState>>,
) -> AppResult<()> {
    let path = resolve_task_file(task_id, index, state.inner()).await?;
    if !path.exists() {
        return Err(AppError::Other(format!(
            "文件不存在或已被移动: {}",
            path.display()
        )));
    }
    open_in_system(&path, false)
}

/// 在文件管理器中打开文件所在文件夹并选中该文件
#[tauri::command]
pub async fn reveal_transfer_file(
    task_id: Uuid,
    index: usize,
    state: State<'_, Arc<AppState>>,
) -> AppResult<()> {
    let path = resolve_task_file(task_id, index, state.inner()).await?;
    if !path.exists() {
        return Err(AppError::Other(format!(
            "文件不存在或已被移动: {}",
            path.display()
        )));
    }
    open_in_system(&path, true)
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
) -> AppResult<Vec<Uuid>> {
    let task_ids = crate::transfer::send_files(request, state.inner().clone()).await?;
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
    /// 关闭窗口时是否最小化到系统托盘（默认 true，保持后台常驻）
    pub minimize_to_tray: bool,
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
        device_name: crate::core::default_device_name(),
        download_path: crate::core::default_download_dir().to_string_lossy().to_string(),
        auto_accept: true,
        upload_limit: 0,
        download_limit: 0,
        enable_notification: true,
        theme: "auto".to_string(),
        minimize_to_tray: true,
    })
}

#[tauri::command]
pub async fn save_settings(settings: Settings, state: State<'_, Arc<AppState>>) -> AppResult<()> {
    if let Some(storage) = get_storage() {
        storage.save_settings(&settings)?;
    }
    state.set_device_name(settings.device_name.clone());
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

/// Event names for frontend communication
pub const TRANSFER_COMPLETED_EVENT: &str = "transfer-completed";
pub const TRANSFER_FAILED_EVENT: &str = "transfer-failed";
pub const INCOMING_TRANSFER_EVENT: &str = "incoming-transfer";

/// 收到他人发来的文件传输时推送给前端的载荷
#[derive(Debug, Serialize, Clone)]
pub struct IncomingTransferPayload {
    pub task_id: Uuid,
    pub sender_name: String,
    pub file_count: u32,
    pub total_size: u64,
}

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

/// Emit transfer completed event and send notification
pub fn emit_transfer_completed(handle: &tauri::AppHandle, task_id: Uuid, device_name: &str) {
    let _ = handle.emit(TRANSFER_COMPLETED_EVENT, task_id);

    // Send system notification
    send_notification(handle, "传输完成", &format!("与 {} 的传输已完成", device_name));
}

/// Emit transfer failed event and send notification
pub fn emit_transfer_failed(handle: &tauri::AppHandle, task_id: Uuid, error: String) {
    let _ = handle.emit(TRANSFER_FAILED_EVENT, serde_json::json!({
        "taskId": task_id,
        "error": error
    }));

    // Send system notification
    send_notification(handle, "传输失败", &format!("文件传输失败：{}", error));
}

/// 收到他人发来的文件传输时：推送事件给前端弹窗，并发送系统通知
pub fn emit_incoming_transfer(
    handle: &tauri::AppHandle,
    task_id: Uuid,
    sender_name: &str,
    file_count: u32,
    total_size: u64,
) {
    let payload = IncomingTransferPayload {
        task_id,
        sender_name: sender_name.to_string(),
        file_count,
        total_size,
    };
    let _ = handle.emit(INCOMING_TRANSFER_EVENT, &payload);

    let body = if file_count <= 1 {
        format!("来自 {} 的文件传输", sender_name)
    } else {
        format!("来自 {} 的 {} 个文件", sender_name, file_count)
    };
    send_notification(handle, "收到文件传输", &body);
}

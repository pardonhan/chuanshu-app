use serde::{Serialize, Deserialize};
use std::sync::Arc;
use tauri::{Emitter, State};
use tokio::runtime::Handle;
use uuid::Uuid;
use crate::core::*;
use crate::network::{DeviceInfo, discovery};
use crate::transfer::TransferTaskInfo;
use crate::storage::{get_storage, Storage};

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

#[tauri::command]
pub async fn discover_device_by_ip(
    ip: String,
    state: State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> AppResult<Option<DeviceInfo>> {
    discovery::discover_device_by_ip(&ip, state.inner().clone(), app_handle).await
}

/// Event names for frontend communication
pub const DEVICE_ONLINE_EVENT: &str = "device-online";
pub const DEVICE_OFFLINE_EVENT: &str = "device-offline";
pub const TRANSFER_PROGRESS_EVENT: &str = "transfer-progress";
pub const TRANSFER_COMPLETED_EVENT: &str = "transfer-completed";
pub const TRANSFER_FAILED_EVENT: &str = "transfer-failed";

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

/// Emit transfer completed event
pub fn emit_transfer_completed(handle: &tauri::AppHandle, task_id: Uuid) {
    let _ = handle.emit(TRANSFER_COMPLETED_EVENT, task_id);
}

/// Emit transfer failed event
pub fn emit_transfer_failed(handle: &tauri::AppHandle, task_id: Uuid, error: String) {
    let _ = handle.emit(TRANSFER_FAILED_EVENT, serde_json::json!({
        "task_id": task_id,
        "error": error
    }));
}

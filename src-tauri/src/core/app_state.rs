use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::Mutex;
use uuid::Uuid;
use tauri::Emitter;
use crate::network::DeviceInfo;
use crate::transfer::TransferTask;

#[derive(Debug, Clone)]
pub struct AppState {
    /// 在线设备列表
    pub devices: Arc<DashMap<Uuid, DeviceInfo>>,
    /// 传输任务列表
    pub transfer_tasks: Arc<DashMap<Uuid, Arc<Mutex<TransferTask>>>>,
    /// 设备ID
    pub device_id: Uuid,
    /// 设备名称
    pub device_name: String,
    /// Tauri app handle for emitting events
    pub app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl AppState {
    pub fn new(device_name: String) -> Self {
        Self {
            devices: Arc::new(DashMap::new()),
            transfer_tasks: Arc::new(DashMap::new()),
            device_id: Uuid::new_v4(),
            device_name,
            app_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Set the app handle
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        self.app_handle.blocking_lock().replace(handle);
    }

    /// Emit an event to the frontend (async version)
    pub async fn emit_async<T: serde::Serialize + Clone>(&self, event: &str, payload: T) -> crate::core::AppResult<()> {
        let handle_guard = self.app_handle.lock().await;
        if let Some(handle) = handle_guard.as_ref() {
            handle.emit(event, payload)
                .map_err(|e| crate::core::AppError::Other(format!("Failed to emit event: {}", e)))?;
        }
        Ok(())
    }

    /// Emit an event to the frontend (sync version using blocking_lock)
    pub fn emit<T: serde::Serialize + Clone>(&self, event: &str, payload: T) -> crate::core::AppResult<()> {
        let handle_guard = self.app_handle.blocking_lock();
        if let Some(handle) = handle_guard.as_ref() {
            handle.emit(event, payload)
                .map_err(|e| crate::core::AppError::Other(format!("Failed to emit event: {}", e)))?;
        }
        Ok(())
    }

    /// Get the app handle
    pub fn get_app_handle(&self) -> Option<tauri::AppHandle> {
        self.app_handle.blocking_lock().clone()
    }
}

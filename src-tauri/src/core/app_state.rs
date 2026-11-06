use std::sync::{Arc, Mutex, RwLock};
use dashmap::DashMap;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;
use tauri::Emitter;
use crate::network::DeviceInfo;
use crate::transfer::TransferTask;

#[derive(Debug, Clone)]
pub struct AppState {
    /// 在线设备列表
    pub devices: Arc<DashMap<Uuid, DeviceInfo>>,
    /// 传输任务列表
    pub transfer_tasks: Arc<DashMap<Uuid, Arc<AsyncMutex<TransferTask>>>>,
    /// 设备ID（持久化）
    pub device_id: Uuid,
    /// 设备名称（可在设置中修改，心跳使用最新值）
    device_name: Arc<RwLock<String>>,
    /// Tauri app handle for emitting events
    app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl AppState {
    pub fn new(device_id: Uuid, device_name: String) -> Self {
        Self {
            devices: Arc::new(DashMap::new()),
            transfer_tasks: Arc::new(DashMap::new()),
            device_id,
            device_name: Arc::new(RwLock::new(device_name)),
            app_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// 当前设备名称
    pub fn device_name(&self) -> String {
        self.device_name
            .read()
            .map(|n| n.clone())
            .unwrap_or_else(|_| "我的设备".to_string())
    }

    /// 更新设备名称
    pub fn set_device_name(&self, name: String) {
        if let Ok(mut guard) = self.device_name.write() {
            *guard = name;
        }
    }

    /// Set the app handle
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        if let Ok(mut guard) = self.app_handle.lock() {
            *guard = Some(handle);
        }
    }

    /// Get the app handle
    pub fn get_app_handle(&self) -> Option<tauri::AppHandle> {
        self.app_handle.lock().ok().and_then(|g| g.clone())
    }

    /// Emit an event to the frontend
    pub fn emit<T: serde::Serialize + Clone>(&self, event: &str, payload: T) -> crate::core::AppResult<()> {
        if let Some(handle) = self.get_app_handle() {
            handle.emit(event, payload)
                .map_err(|e| crate::core::AppError::Other(format!("Failed to emit event: {}", e)))?;
        }
        Ok(())
    }

    /// Emit an event to the frontend (async-compatible alias)
    pub async fn emit_async<T: serde::Serialize + Clone>(&self, event: &str, payload: T) -> crate::core::AppResult<()> {
        self.emit(event, payload)
    }
}

use crate::core::MAX_CONCURRENT_TRANSFERS;
use crate::network::DeviceInfo;
use crate::transfer::TransferTask;
use dashmap::DashMap;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::oneshot;
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

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
    /// Incoming transfer requests waiting for a frontend decision.
    pub pending_transfer_requests: Arc<DashMap<Uuid, oneshot::Sender<bool>>>,
    /// SHA-256 fingerprint of the local QUIC certificate.
    pub certificate_fingerprint: Arc<Mutex<Option<String>>>,
    /// 限制同时进行的发送任务数量。
    pub transfer_semaphore: Arc<Semaphore>,
}

impl AppState {
    pub fn new(device_name: String) -> Self {
        Self {
            devices: Arc::new(DashMap::new()),
            transfer_tasks: Arc::new(DashMap::new()),
            device_id: Uuid::new_v4(),
            device_name,
            app_handle: Arc::new(Mutex::new(None)),
            pending_transfer_requests: Arc::new(DashMap::new()),
            certificate_fingerprint: Arc::new(Mutex::new(None)),
            transfer_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_TRANSFERS)),
        }
    }

    pub async fn set_certificate_fingerprint(&self, fingerprint: String) {
        self.certificate_fingerprint
            .lock()
            .await
            .replace(fingerprint);
    }

    pub async fn certificate_fingerprint(&self) -> Option<String> {
        self.certificate_fingerprint.lock().await.clone()
    }

    /// Set the app handle (startup only; never blocks a runtime thread).
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        if let Ok(mut guard) = self.app_handle.try_lock() {
            guard.replace(handle);
        }
    }

    /// Emit an event to the frontend (async version)
    pub async fn emit_async<T: serde::Serialize + Clone>(
        &self,
        event: &str,
        payload: T,
    ) -> crate::core::AppResult<()> {
        let handle_guard = self.app_handle.lock().await;
        if let Some(handle) = handle_guard.as_ref() {
            handle.emit(event, payload).map_err(|e| {
                crate::core::AppError::Other(format!("Failed to emit event: {}", e))
            })?;
        }
        Ok(())
    }

    /// Get the app handle
    pub async fn get_app_handle(&self) -> Option<tauri::AppHandle> {
        self.app_handle.lock().await.clone()
    }
}

use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::Mutex;
use uuid::Uuid;
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
}

impl AppState {
    pub fn new(device_name: String) -> Self {
        Self {
            devices: Arc::new(DashMap::new()),
            transfer_tasks: Arc::new(DashMap::new()),
            device_id: Uuid::new_v4(),
            device_name,
        }
    }
}

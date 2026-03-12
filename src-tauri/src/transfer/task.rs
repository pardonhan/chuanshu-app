use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;
use uuid::Uuid;

use crate::network::protocol::FileMetadata;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransferStatus {
    Pending,
    Transferring,
    Paused,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransferType {
    Send,
    Receive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferTask {
    pub task_id: Uuid,
    pub peer_device_id: Uuid,
    pub peer_device_name: String,
    pub transfer_type: TransferType,
    pub status: TransferStatus,
    pub total_size: u64,
    pub transferred_size: u64,
    pub file_count: u32,
    pub files: Vec<FileMetadata>,
    pub current_file_index: u32,
    pub current_file: String,
    pub speed: u64,
    pub error_message: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub save_path: Option<PathBuf>,
    pub source_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferTaskInfo {
    pub task_id: Uuid,
    pub peer_device_name: String,
    pub transfer_type: TransferType,
    pub status: TransferStatus,
    pub total_size: u64,
    pub transferred_size: u64,
    pub file_count: u32,
    pub current_file: String,
    pub speed: u64,
    pub progress: f64,
    pub error_message: Option<String>,
    pub created_at: u64,
}

impl TransferTask {
    pub fn new(
        task_id: Uuid,
        peer_device_id: Uuid,
        peer_device_name: String,
        transfer_type: TransferType,
        total_size: u64,
        file_count: u32,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            task_id,
            peer_device_id,
            peer_device_name,
            transfer_type,
            status: TransferStatus::Pending,
            total_size,
            transferred_size: 0,
            file_count,
            files: Vec::new(),
            current_file_index: 0,
            current_file: String::new(),
            speed: 0,
            error_message: None,
            created_at: now,
            updated_at: now,
            save_path: None,
            source_paths: Vec::new(),
        }
    }

    pub fn update_progress(&mut self, transferred: u64, speed: u64) {
        self.transferred_size = transferred;
        self.speed = speed;
        self.updated_at = SystemTime::now();
    }

    pub fn set_status(&mut self, status: TransferStatus) {
        self.status = status;
        self.updated_at = SystemTime::now();
    }

    pub fn set_current_file(&mut self, file_name: &str, index: u32) {
        self.current_file = file_name.to_string();
        self.current_file_index = index;
        self.updated_at = SystemTime::now();
    }

    pub fn set_error(&mut self, error: String) {
        self.error_message = Some(error);
        self.status = TransferStatus::Failed;
        self.updated_at = SystemTime::now();
    }

    pub fn progress(&self) -> f64 {
        if self.total_size == 0 {
            0.0
        } else {
            self.transferred_size as f64 / self.total_size as f64 * 100.0
        }
    }
}

impl From<&TransferTask> for TransferTaskInfo {
    fn from(task: &TransferTask) -> Self {
        let created_at = task.created_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            task_id: task.task_id,
            peer_device_name: task.peer_device_name.clone(),
            transfer_type: task.transfer_type.clone(),
            status: task.status.clone(),
            total_size: task.total_size,
            transferred_size: task.transferred_size,
            file_count: task.file_count,
            current_file: task.current_file.clone(),
            speed: task.speed,
            progress: task.progress(),
            error_message: task.error_message.clone(),
            created_at,
        }
    }
}

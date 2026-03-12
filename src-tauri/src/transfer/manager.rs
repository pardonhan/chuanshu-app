use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::time::interval;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::core::{AppResult, AppState, MAX_CONCURRENT_TRANSFERS};
use crate::ipc::SendFilesRequest;
use crate::network::device::DeviceInfo;
use crate::network::protocol::{FileMetadata, TransferRequest};
use crate::network::quic_client::{request_transfer, init_quic_client};
use crate::transfer::{
    TransferTask, TransferTaskInfo, TransferType, TransferStatus,
    file_chunk::{FileChunker, calculate_file_hash},
    resume::ResumeInfo,
};

/// Transfer manager for handling file transfers
pub struct TransferManager {
    app_state: Arc<AppState>,
    active_transfers: Arc<Mutex<Vec<Uuid>>>,
}

impl TransferManager {
    /// Create a new transfer manager
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self {
            app_state,
            active_transfers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Send files to one or more devices
    pub async fn send_files(
        &self,
        request: SendFilesRequest,
    ) -> AppResult<Vec<Uuid>> {
        let mut task_ids = Vec::new();

        // Collect file information
        let files_info = self.collect_files(&request.file_paths).await?;
        let total_size: u64 = files_info.iter().map(|f| f.file_size).sum();
        let file_count = files_info.len() as u32;

        // Create transfer tasks for each device
        for device_id in request.device_ids {
            let task_id = Uuid::new_v4();

            // Get device info
            let device = self.app_state.devices.get(&device_id)
                .ok_or_else(|| crate::core::AppError::DeviceNotFound(
                    format!("Device {} not found", device_id)
                ))?;

            // Create transfer request
            let transfer_request = TransferRequest {
                task_id,
                sender_id: self.app_state.device_id,
                sender_name: self.app_state.device_name.clone(),
                file_count,
                total_size,
                files: files_info.clone(),
                resume: false,
            };

            // Create task
            let task = TransferTask::new(
                task_id,
                device_id,
                device.device_name.clone(),
                TransferType::Send,
                total_size,
                file_count,
            );

            // Store task
            self.app_state.transfer_tasks.insert(
                task_id,
                Arc::new(Mutex::new(task))
            );

            task_ids.push(task_id);

            // Initiate transfer
            let device_addr = SocketAddr::new(device.ip_address, device.quic_port);
            let app_state = self.app_state.clone();

            tokio::spawn(async move {
                if let Err(e) = initiate_transfer(
                    device_addr,
                    device_id,
                    transfer_request,
                    app_state.clone(),
                ).await {
                    log::error!("Failed to initiate transfer: {}", e);

                    // Update task status
                    if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                        let mut task = entry.lock().await;
                        task.set_error(e.to_string());
                    }
                }
            });
        }

        Ok(task_ids)
    }

    /// Collect file information from paths
    async fn collect_files(&self, paths: &[String]) -> AppResult<Vec<FileMetadata>> {
        let mut files = Vec::new();
        let mut file_id: u64 = 0;

        for path_str in paths {
            let path = PathBuf::from(path_str);

            if path.is_file() {
                let metadata = self.get_file_metadata(&path, &PathBuf::new(), file_id).await?;
                files.push(metadata);
                file_id += 1;
            } else if path.is_dir() {
                // Walk directory
                let base_path = path.parent().unwrap_or(&path);

                for entry in WalkDir::new(&path).follow_links(true) {
                    let entry = entry.map_err(|e| {
                        let io_err = e.io_error()
                            .map(|err| std::io::Error::new(err.kind(), err.to_string()))
                            .unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "WalkDir error"));
                        crate::core::AppError::Io(io_err)
                    })?;

                    if entry.file_type().is_file() {
                        let relative_path = entry.path()
                            .strip_prefix(base_path)
                            .unwrap_or(entry.path())
                            .parent()
                            .unwrap_or(Path::new(""))
                            .to_path_buf();

                        let file_metadata = self.get_file_metadata(
                            entry.path(),
                            &relative_path,
                            file_id
                        ).await?;

                        files.push(file_metadata);
                        file_id += 1;
                    }
                }
            }
        }

        Ok(files)
    }

    /// Get metadata for a single file
    async fn get_file_metadata(
        &self,
        path: &Path,
        relative_path: &Path,
        file_id: u64,
    ) -> AppResult<FileMetadata> {
        let metadata = tokio::fs::metadata(path).await?;
        let file_name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let modified_time = metadata.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Calculate checksum for files smaller than 100MB
        let checksum = if metadata.len() < 100 * 1024 * 1024 {
            calculate_file_hash(path).await.ok()
        } else {
            None
        };

        Ok(FileMetadata {
            file_id,
            relative_path: relative_path.to_string_lossy().to_string(),
            file_name,
            file_size: metadata.len(),
            modified_time,
            checksum,
            is_directory: false,
        })
    }

    /// Cancel a transfer
    pub async fn cancel_transfer(&self, task_id: Uuid) -> AppResult<()> {
        if let Some(entry) = self.app_state.transfer_tasks.get(&task_id) {
            let mut task = entry.lock().await;
            task.set_status(TransferStatus::Canceled);

            // TODO: Notify peer
            log::info!("Transfer {} canceled", task_id);
        }

        // Remove from active transfers
        let mut active = self.active_transfers.lock().await;
        active.retain(|&id| id != task_id);

        Ok(())
    }

    /// Pause a transfer
    pub async fn pause_transfer(&self, task_id: Uuid) -> AppResult<()> {
        if let Some(entry) = self.app_state.transfer_tasks.get(&task_id) {
            let mut task = entry.lock().await;

            if task.status == TransferStatus::Transferring {
                task.set_status(TransferStatus::Paused);
                log::info!("Transfer {} paused", task_id);
            }
        }

        Ok(())
    }

    /// Resume a transfer
    pub async fn resume_transfer(&self, task_id: Uuid) -> AppResult<()> {
        if let Some(entry) = self.app_state.transfer_tasks.get(&task_id) {
            let mut task = entry.lock().await;

            if task.status == TransferStatus::Paused {
                task.set_status(TransferStatus::Transferring);
                log::info!("Transfer {} resumed", task_id);

                // TODO: Re-initiate the transfer with resume info
            }
        }

        Ok(())
    }

    /// Get all transfer tasks
    pub async fn get_all_tasks(&self) -> Vec<TransferTaskInfo> {
        let mut tasks = Vec::new();

        for entry in self.app_state.transfer_tasks.iter() {
            let task = entry.lock().await;
            tasks.push(TransferTaskInfo::from(&*task));
        }

        // Sort by created_at descending
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        tasks
    }

    /// Clean up completed/canceled tasks older than specified duration
    pub async fn cleanup_old_tasks(&self, max_age: Duration) -> u64 {
        let now = std::time::SystemTime::now();
        let mut removed = 0u64;

        let to_remove: Vec<Uuid> = self.app_state.transfer_tasks
            .iter()
            .filter_map(|entry| {
                let task = entry.value().blocking_lock();
                if matches!(task.status, TransferStatus::Completed | TransferStatus::Canceled | TransferStatus::Failed) {
                    if let Ok(age) = now.duration_since(task.updated_at) {
                        if age > max_age {
                            return Some(*entry.key());
                        }
                    }
                }
                None
            })
            .collect();

        for task_id in to_remove {
            if self.app_state.transfer_tasks.remove(&task_id).is_some() {
                removed += 1;
            }
        }

        removed
    }
}

/// Initiate a transfer to a peer
async fn initiate_transfer(
    peer_addr: SocketAddr,
    peer_device_id: Uuid,
    request: TransferRequest,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    // Initialize QUIC client if needed
    init_quic_client().await.ok();

    // Send transfer request
    let response = request_transfer(peer_addr, peer_device_id, request.clone(), app_state.clone()).await?;

    if !response.accepted {
        let reason = response.reason.unwrap_or_else(|| "Transfer rejected".to_string());
        return Err(crate::core::AppError::Other(reason));
    }

    // Update task status
    if let Some(entry) = app_state.transfer_tasks.get(&request.task_id) {
        let mut task = entry.lock().await;
        task.set_status(TransferStatus::Transferring);
    }

    log::info!("Transfer {} accepted by peer", request.task_id);

    // TODO: Start actual file transfer
    // This would involve:
    // 1. Opening data streams
    // 2. Sending file chunks
    // 3. Progress tracking
    // 4. Handling pause/resume/cancel

    Ok(())
}

/// Background task for progress updates
pub async fn start_progress_updater(app_state: Arc<AppState>) {
    let mut ticker = interval(Duration::from_millis(500));

    loop {
        ticker.tick().await;

        for entry in app_state.transfer_tasks.iter() {
            let mut task = entry.lock().await;

            if task.status == TransferStatus::Transferring {
                // Calculate speed based on recent progress
                // This is a simplified calculation
                // TODO: Implement proper speed calculation

                // Emit progress event to frontend
                let info = TransferTaskInfo::from(&*task);
                // Event emission would go here
            }
        }
    }
}

/// Public function to send files
pub async fn send_files(
    request: SendFilesRequest,
    state: Arc<AppState>,
    _rt_handle: Handle,
) -> AppResult<Vec<Uuid>> {
    let manager = TransferManager::new(state);
    manager.send_files(request).await
}

/// Public function to cancel transfer
pub async fn cancel_transfer(task_id: Uuid, state: Arc<AppState>) -> AppResult<()> {
    let manager = TransferManager::new(state);
    manager.cancel_transfer(task_id).await
}

/// Public function to pause transfer
pub async fn pause_transfer(task_id: Uuid, state: Arc<AppState>) -> AppResult<()> {
    let manager = TransferManager::new(state);
    manager.pause_transfer(task_id).await
}

/// Public function to resume transfer
pub async fn resume_transfer(task_id: Uuid, state: Arc<AppState>) -> AppResult<()> {
    let manager = TransferManager::new(state);
    manager.resume_transfer(task_id).await
}

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::time::interval;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::core::{AppResult, AppState, DEFAULT_CHUNK_SIZE};
use crate::ipc::SendFilesRequest;
use crate::network::protocol::{
    FileMetadata, TransferRequest, DataPacket, ChunkMetadata,
};
use crate::network::quic_client::{request_transfer, init_quic_client};
use crate::transfer::{
    TransferTask, TransferTaskInfo, TransferType, TransferStatus,
    file_chunk::{calculate_crc32, calculate_file_hash},
    rate_limiter::SharedRateLimiter,
    resume::{ResumeInfo, ResumeManager},
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
            source_path: path.to_string_lossy().to_string(),
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

                // Load resume info and re-initiate transfer
                if let Some(storage) = crate::storage::get_storage() {
                    let resume_manager = ResumeManager::new(&*storage);
                    if let Ok(Some(resume_info)) = resume_manager.load_resume(task_id) {
                        let app_state = self.app_state.clone();
                        tokio::spawn(async move {
                            match resume_transfer_task(task_id, resume_info, app_state).await {
                                Ok(_) => log::info!("Resumed transfer {} completed", task_id),
                                Err(e) => log::error!("Resumed transfer {} failed: {}", task_id, e),
                            }
                        });
                    }
                }
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

    // Get upload limit from settings
    let upload_limit = {
        let storage = crate::storage::get_storage();
        storage
            .and_then(|s| s.load_settings().ok().flatten())
            .map(|s| s.upload_limit)
            .unwrap_or(0)
    };

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
        task.files = request.files.clone();
    }

    log::info!("Transfer {} accepted by peer", request.task_id);

    // Start actual file transfer
    let task_id = request.task_id;
    let files = request.files.clone();
    let total_size = request.total_size;
    let file_count = request.file_count;
    let sender_id = request.sender_id;
    let sender_name = request.sender_name.clone();
    let app_state_clone = app_state.clone();

    tokio::spawn(async move {
        match send_files_data(
            peer_addr,
            task_id,
            files,
            total_size,
            file_count,
            sender_id,
            &sender_name,
            app_state_clone,
            upload_limit,
        ).await {
            Ok(_) => {
                log::info!("Transfer {} completed successfully", task_id);
                if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                    let mut task = entry.lock().await;
                    task.set_status(TransferStatus::Completed);

                    // Save to transfer history
                    if let Some(storage) = crate::storage::get_storage() {
                        let file_names = task.files
                            .iter()
                            .map(|f| f.file_name.clone())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let _ = storage.add_transfer_history(
                            task_id,
                            task.peer_device_id,
                            &task.peer_device_name,
                            "send",
                            "completed",
                            task.total_size,
                            task.file_count,
                            Some(&file_names),
                        );
                    }

                    // Send completion notification
                    let handle = app_state.get_app_handle();
                    if let Some(h) = handle {
                        let _ = crate::ipc::emit_transfer_completed(&h, task_id, &task.peer_device_name);
                    }
                }
            }
            Err(e) => {
                log::error!("Transfer {} failed: {}", task_id, e);
                if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                    let mut task = entry.lock().await;
                    task.set_error(e.to_string());

                    // Save to transfer history
                    if let Some(storage) = crate::storage::get_storage() {
                        let file_names = task.files
                            .iter()
                            .map(|f| f.file_name.clone())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let _ = storage.add_transfer_history(
                            task_id,
                            task.peer_device_id,
                            &task.peer_device_name,
                            "send",
                            "failed",
                            task.total_size,
                            task.file_count,
                            Some(&file_names),
                        );
                    }

                    // Send failure notification
                    let handle = app_state.get_app_handle();
                    if let Some(h) = handle {
                        let _ = crate::ipc::emit_transfer_failed(&h, task_id, e.to_string());
                    }
                }
            }
        }
    });

    Ok(())
}

/// Send actual file data to peer
async fn send_files_data(
    peer_addr: SocketAddr,
    task_id: Uuid,
    files: Vec<FileMetadata>,
    _total_size: u64,
    _file_count: u32,
    _sender_id: Uuid,
    _sender_name: &str,
    app_state: Arc<AppState>,
    upload_limit: u32,
) -> AppResult<()> {
    // Create a temporary endpoint for connection
    let client_config = crate::network::connection::create_client_config()?;
    let bind_addr: SocketAddr = "0.0.0.0:0".parse()
        .map_err(|e| crate::core::AppError::Other(format!("Invalid bind address: {}", e)))?;
    let mut endpoint = quinn::Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(client_config);

    // Connect to peer (use sender_id as temporary device_id)
    let connecting = endpoint.connect(peer_addr, "chuanshu.local")
        .map_err(|e| crate::core::AppError::Other(format!("Failed to connect: {}", e)))?;
    let connection = match tokio::time::timeout(Duration::from_secs(10), connecting).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(e)) => return Err(crate::core::AppError::Network(format!("Connection failed: {}", e))),
        Err(_) => return Err(crate::core::AppError::Other("Connection timeout".to_string())),
    };

    // Open stream for data transfer
    let (mut send, mut recv) = connection.open_bi().await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to open stream: {}", e)))?;

    // Create rate limiter for upload
    let rate_limiter = SharedRateLimiter::new(upload_limit);

    let mut transferred_size: u64 = 0;
    let mut last_update_time = std::time::Instant::now();
    let mut last_transferred = 0u64;

    // Send each file
    for (file_index, file_meta) in files.iter().enumerate() {
        // Update current file in task
        if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
            let mut task = entry.lock().await;
            task.set_current_file(&file_meta.file_name, file_index as u32);
        }

        // Send file header
        let file_header = FileHeader {
            task_id,
            file_id: file_meta.file_id,
            file_name: file_meta.file_name.clone(),
            file_size: file_meta.file_size,
            relative_path: file_meta.relative_path.clone(),
            checksum: file_meta.checksum.clone(),
        };

        // Send header with length prefix
        let header_data = bincode::serialize(&file_header)?;
        send.write_all(&header_data.len().to_be_bytes()).await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write header: {}", e)))?;
        send.write_all(&header_data).await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write header data: {}", e)))?;

        // Read and send file chunks using source_path
        let file_path = Path::new(&file_meta.source_path);
        let mut file = tokio::fs::File::open(file_path).await?;

        let chunk_size = DEFAULT_CHUNK_SIZE as u64;
        let mut offset: u64 = 0;
        let mut chunk_index: u64 = 0;
        let total_chunks = ((file_meta.file_size + chunk_size - 1) / chunk_size).max(1);

        loop {
            // Check if task is paused or canceled
            if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                let task = entry.lock().await;
                if task.status == TransferStatus::Canceled {
                    return Err(crate::core::AppError::Other("Transfer canceled".to_string()));
                }
                if task.status == TransferStatus::Paused {
                    // Wait until resumed
                    drop(task);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            }

            // Read chunk
            let mut buffer = vec![0u8; chunk_size.min(file_meta.file_size - offset) as usize];
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            buffer.truncate(n);

            // Calculate checksum
            let checksum = calculate_crc32(&buffer);

            // Create chunk metadata
            let chunk_meta = ChunkMetadata {
                task_id,
                file_id: file_meta.file_id,
                chunk_index,
                total_chunks,
                chunk_size: n as u32,
                offset,
                checksum,
            };

            // Create data packet
            let packet = DataPacket {
                metadata: chunk_meta,
                data: buffer,
            };

            // Send packet
            let packet_data = bincode::serialize(&packet)?;
            send.write_all(&packet_data.len().to_be_bytes()).await
                .map_err(|e| crate::core::AppError::Network(format!("Failed to write packet: {}", e)))?;
            send.write_all(&packet_data).await
                .map_err(|e| crate::core::AppError::Network(format!("Failed to write packet data: {}", e)))?;

            // Apply rate limiting after sending data
            if upload_limit > 0 {
                rate_limiter.consume_and_wait(packet_data.len() as u64).await;
            }

            offset += n as u64;
            chunk_index += 1;
            transferred_size += n as u64;

            // Emit progress every 100ms
            let now = std::time::Instant::now();
            if now.duration_since(last_update_time) >= Duration::from_millis(100) {
                let elapsed = now.duration_since(last_update_time).as_secs_f64();
                let speed = if elapsed > 0.0 {
                    ((transferred_size - last_transferred) as f64 / elapsed) as u64
                } else {
                    0
                };

                // Update task and emit event
                if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                    let mut task = entry.lock().await;
                    task.update_progress(transferred_size, speed);

                    // Emit event to frontend
                    let info = TransferTaskInfo::from(&*task);
                    let _ = app_state.emit_async("transfer-progress", &info).await;
                }

                last_update_time = now;
                last_transferred = transferred_size;
            }
        }
    }

    // Send transfer complete marker
    send.write_all(&0u32.to_be_bytes()).await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write complete marker: {}", e)))?;
    send.finish().await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to finish stream: {}", e)))?;

    // Wait for acknowledgment
    let mut ack_buf = [0u8; 1];
    let _ = tokio::time::timeout(Duration::from_secs(5), recv.read(&mut ack_buf)).await;

    Ok(())
}

/// File header for each file in transfer
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FileHeader {
    task_id: Uuid,
    file_id: u64,
    file_name: String,
    file_size: u64,
    relative_path: String,
    checksum: Option<String>,
}

/// Background task for progress updates
pub async fn start_progress_updater(app_state: Arc<AppState>) {
    let mut ticker = interval(Duration::from_millis(500));

    loop {
        ticker.tick().await;

        for entry in app_state.transfer_tasks.iter() {
            let task = entry.lock().await;

            if task.status == TransferStatus::Transferring {
                // Emit progress event to frontend
                let info = TransferTaskInfo::from(&*task);
                let _ = app_state.emit_async("transfer-progress", &info).await;
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

/// Resume a transfer task from saved state
async fn resume_transfer_task(
    task_id: Uuid,
    resume_info: ResumeInfo,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    // Get peer address from stored device info
    let peer_device = app_state.devices.get(&resume_info.peer_device_id)
        .ok_or_else(|| crate::core::AppError::DeviceNotFound(
            format!("Peer device {} not found", resume_info.peer_device_id)
        ))?;

    let peer_addr = SocketAddr::new(peer_device.ip_address, peer_device.quic_port);
    drop(peer_device);

    // Get upload limit from settings
    let upload_limit = {
        let storage = crate::storage::get_storage();
        storage
            .and_then(|s| s.load_settings().ok().flatten())
            .map(|s| s.upload_limit)
            .unwrap_or(0)
    };

    // Collect files from resume info
    let files: Vec<FileMetadata> = resume_info.files.values()
        .map(|f| f.metadata.clone())
        .collect();

    log::info!("Resuming transfer {} to {} at {}", task_id, resume_info.peer_device_name, peer_addr);

    // Connect and send files with resume support
    match send_files_data_with_resume(
        peer_addr,
        task_id,
        files,
        resume_info,
        upload_limit,
        app_state.clone(),
    ).await {
        Ok(_) => {
            log::info!("Resumed transfer {} completed successfully", task_id);
            if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                let mut task = entry.lock().await;
                task.set_status(TransferStatus::Completed);
            }
            // Clean up resume info
            if let Some(storage) = crate::storage::get_storage() {
                let _ = storage.delete_resume_info(task_id);
            }
        }
        Err(e) => {
            log::error!("Resumed transfer {} failed: {}", task_id, e);
            if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                let mut task = entry.lock().await;
                task.set_error(e.to_string());
            }
        }
    }

    Ok(())
}

/// Send file data with resume support (sender side)
async fn send_files_data_with_resume(
    peer_addr: SocketAddr,
    task_id: Uuid,
    files: Vec<FileMetadata>,
    _resume_info: ResumeInfo,
    upload_limit: u32,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    // Create a temporary endpoint for connection
    let client_config = crate::network::connection::create_client_config()?;
    let bind_addr: SocketAddr = "0.0.0.0:0".parse()
        .map_err(|e| crate::core::AppError::Other(format!("Invalid bind address: {}", e)))?;
    let mut endpoint = quinn::Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(client_config);

    // Connect to peer
    let connecting = endpoint.connect(peer_addr, "chuanshu.local")
        .map_err(|e| crate::core::AppError::Other(format!("Failed to connect: {}", e)))?;
    let connection = match tokio::time::timeout(Duration::from_secs(10), connecting).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(e)) => return Err(crate::core::AppError::Network(format!("Connection failed: {}", e))),
        Err(_) => return Err(crate::core::AppError::Other("Connection timeout".to_string())),
    };

    // Open stream for data transfer
    let (mut send, mut recv) = connection.open_bi().await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to open stream: {}", e)))?;

    // Create rate limiter for upload
    let rate_limiter = SharedRateLimiter::new(upload_limit);

    let mut transferred_size: u64 = 0;
    let mut last_update_time = std::time::Instant::now();
    let mut last_transferred = 0u64;

    // Send each file
    for (file_index, file_meta) in files.iter().enumerate() {
        // Update current file in task
        if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
            let mut task = entry.lock().await;
            task.set_current_file(&file_meta.file_name, file_index as u32);
        }

        // Send file header
        let file_header = FileHeader {
            task_id,
            file_id: file_meta.file_id,
            file_name: file_meta.file_name.clone(),
            file_size: file_meta.file_size,
            relative_path: file_meta.relative_path.clone(),
            checksum: file_meta.checksum.clone(),
        };

        // Send header with length prefix
        let header_data = bincode::serialize(&file_header)?;
        send.write_all(&header_data.len().to_be_bytes()).await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write header: {}", e)))?;
        send.write_all(&header_data).await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write header data: {}", e)))?;

        // Read and send file chunks using source_path
        let file_path = Path::new(&file_meta.source_path);
        let mut file = tokio::fs::File::open(file_path).await?;

        let chunk_size = DEFAULT_CHUNK_SIZE as u64;
        let mut offset: u64 = 0;
        let mut chunk_index: u64 = 0;
        let total_chunks = ((file_meta.file_size + chunk_size - 1) / chunk_size).max(1);

        loop {
            // Check if task is paused or canceled
            if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                let task = entry.lock().await;
                if task.status == TransferStatus::Canceled {
                    return Err(crate::core::AppError::Other("Transfer canceled".to_string()));
                }
                if task.status == TransferStatus::Paused {
                    drop(task);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            }

            // Read chunk
            let mut buffer = vec![0u8; chunk_size.min(file_meta.file_size - offset) as usize];
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            buffer.truncate(n);

            // Calculate checksum
            let checksum = calculate_crc32(&buffer);

            // Create chunk metadata
            let chunk_meta = ChunkMetadata {
                task_id,
                file_id: file_meta.file_id,
                chunk_index,
                total_chunks,
                chunk_size: n as u32,
                offset,
                checksum,
            };

            // Create data packet
            let packet = DataPacket {
                metadata: chunk_meta,
                data: buffer,
            };

            // Send packet
            let packet_data = bincode::serialize(&packet)?;
            send.write_all(&packet_data.len().to_be_bytes()).await
                .map_err(|e| crate::core::AppError::Network(format!("Failed to write packet: {}", e)))?;
            send.write_all(&packet_data).await
                .map_err(|e| crate::core::AppError::Network(format!("Failed to write packet data: {}", e)))?;

            // Apply rate limiting after sending data
            if upload_limit > 0 {
                rate_limiter.consume_and_wait(packet_data.len() as u64).await;
            }

            offset += n as u64;
            chunk_index += 1;
            transferred_size += n as u64;

            // Emit progress every 100ms
            let now = std::time::Instant::now();
            if now.duration_since(last_update_time) >= Duration::from_millis(100) {
                let elapsed = now.duration_since(last_update_time).as_secs_f64();
                let speed = if elapsed > 0.0 {
                    ((transferred_size - last_transferred) as f64 / elapsed) as u64
                } else {
                    0
                };

                // Update task and emit event
                if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                    let mut task = entry.lock().await;
                    task.update_progress(transferred_size, speed);

                    // Emit event to frontend
                    let info = TransferTaskInfo::from(&*task);
                    let _ = app_state.emit_async("transfer-progress", &info).await;
                }

                last_update_time = now;
                last_transferred = transferred_size;
            }
        }
    }

    // Send transfer complete marker
    send.write_all(&0u32.to_be_bytes()).await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write complete marker: {}", e)))?;
    send.finish().await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to finish stream: {}", e)))?;

    // Wait for acknowledgment
    let mut ack_buf = [0u8; 1];
    let _ = tokio::time::timeout(Duration::from_secs(5), recv.read(&mut ack_buf)).await;

    Ok(())
}

use quinn::{Connection, Endpoint};
use std::io::{Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use crate::core::{AppResult, AppState, QUIC_DEFAULT_PORT};
use crate::network::auth::authenticate_as_server;
use crate::network::connection::{
    create_server_config, generate_self_signed_cert, init_connection_pool,
};
use crate::network::protocol::{
    ControlMessage, ControlMessageType, DataPacket, DataTransferReady, DataTransferStart,
    FileHeader, TransferRequest, TransferResponse,
};
use crate::transfer::file_chunk::calculate_crc32;
use crate::transfer::rate_limiter::SharedRateLimiter;
use crate::transfer::{TransferStatus, TransferTask, TransferTaskInfo, TransferType};

/// QUIC server handle
pub struct QuicServer {
    endpoint: Endpoint,
    port: u16,
    certificate_fingerprint: String,
}

impl QuicServer {
    /// Create and bind a new QUIC server
    pub async fn new(port: u16) -> AppResult<Self> {
        // Generate self-signed certificate
        let (cert, key) = generate_self_signed_cert()?;
        let certificate_fingerprint = crate::network::auth::calculate_cert_fingerprint(&cert.0);
        let server_config = create_server_config(cert, key)?;

        // Bind to address
        let addr: SocketAddr = format!("0.0.0.0:{}", port)
            .parse()
            .map_err(|e| crate::core::AppError::Other(format!("Invalid address: {}", e)))?;

        let endpoint = Endpoint::server(server_config, addr)?;
        let actual_port = endpoint.local_addr()?.port();

        log::info!("QUIC server bound to port {}", actual_port);

        Ok(Self {
            endpoint,
            port: actual_port,
            certificate_fingerprint,
        })
    }

    /// Get the server port
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn certificate_fingerprint(&self) -> &str {
        &self.certificate_fingerprint
    }

    /// Stop the server
    pub fn stop(&self) {
        self.endpoint.close(0u32.into(), b"server shutdown");
        log::info!("QUIC server stopped");
    }
}

/// Handle an incoming QUIC connection
async fn handle_incoming_connection(
    connection: Connection,
    app_state: Arc<AppState>,
    pool: Arc<crate::network::connection::ConnectionPool>,
) -> AppResult<()> {
    let remote_addr = connection.remote_address();
    log::info!("New QUIC connection from {}", remote_addr);

    // Accept the first stream for authentication handshake
    let (send, recv) = match timeout(Duration::from_secs(10), connection.accept_bi()).await {
        Ok(Ok(streams)) => streams,
        Ok(Err(e)) => {
            return Err(crate::core::AppError::Network(format!(
                "Failed to accept stream: {}",
                e
            )))
        }
        Err(_) => {
            return Err(crate::core::AppError::Other(
                "Connection timeout".to_string(),
            ))
        }
    };

    // Perform server authentication
    let peer_device_id = match authenticate_as_server(
        send,
        recv,
        app_state.device_id,
        app_state.device_name.clone(),
        remote_addr,
        app_state.clone(),
    )
    .await
    {
        Ok(device_id) => {
            log::info!("Authenticated peer device {} at {}", device_id, remote_addr);
            device_id
        }
        Err(e) => {
            log::error!("Authentication failed: {}", e);
            return Err(e);
        }
    };

    // Add to connection pool with authenticated device ID
    let peer_conn = pool.add_incoming(peer_device_id, connection.clone());
    peer_conn.mark_authenticated();

    // Start monitoring the connection for keep-alive and disconnect detection
    peer_conn.start_monitoring(pool.clone());

    // Handle subsequent streams
    loop {
        match timeout(Duration::from_secs(30), connection.accept_bi()).await {
            Ok(Ok((send, recv))) => {
                let app_state = app_state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_stream(send, recv, peer_device_id, app_state).await {
                        log::error!("Stream handler error: {}", e);
                    }
                });
            }
            Ok(Err(e)) => {
                log::debug!("Connection closed: {}", e);
                break;
            }
            Err(_) => {
                log::debug!("Connection accept timeout");
                continue;
            }
        }
    }

    pool.remove(&peer_device_id);
    log::info!("QUIC connection from {} closed", remote_addr);

    Ok(())
}

/// Handle a bidirectional stream
async fn handle_stream(
    send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    peer_device_id: Uuid,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    // Read message length
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await.map_err(|e| {
        crate::core::AppError::Other(format!("Failed to read message length: {}", e))
    })?;
    let msg_len = u32::from_be_bytes(len_buf) as usize;

    if msg_len > 10 * 1024 * 1024 {
        // Max 10MB message
        return Err(crate::core::AppError::Other(
            "Message too large".to_string(),
        ));
    }

    // Read message data
    let mut msg_buf = vec![0u8; msg_len];
    recv.read_exact(&mut msg_buf)
        .await
        .map_err(|e| crate::core::AppError::Other(format!("Failed to read message: {}", e)))?;

    // Parse control message
    let message = ControlMessage::from_bytes(&msg_buf)?;

    match message.message_type {
        ControlMessageType::TransferRequest => {
            handle_transfer_request(message, send, recv, peer_device_id, app_state).await?;
        }
        ControlMessageType::DataTransferStart => {
            handle_data_transfer_start(message, send, recv, peer_device_id, app_state).await?;
        }
        ControlMessageType::CancelTransfer => {
            let _ = recv; // Consume recv stream
            handle_cancel_transfer(message, app_state).await?;
        }
        ControlMessageType::PauseTransfer => {
            let _ = recv; // Consume recv stream
            handle_pause_transfer(message, app_state).await?;
        }
        ControlMessageType::ResumeTransfer => {
            let _ = recv; // Consume recv stream
            handle_resume_transfer(message, app_state).await?;
        }
        _ => {
            log::debug!(
                "Received unhandled message type: {:?}",
                message.message_type
            );
        }
    }

    Ok(())
}

/// Handle a transfer request
async fn handle_transfer_request(
    message: ControlMessage,
    mut send: quinn::SendStream,
    recv: quinn::RecvStream,
    peer_device_id: Uuid,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    let request: TransferRequest = message.payload()?;

    if request.sender_id != peer_device_id {
        return Err(crate::core::AppError::Other(
            "Transfer sender does not match authenticated peer".to_string(),
        ));
    }

    log::info!(
        "Received transfer request from {}: {} files, {} bytes",
        request.sender_name,
        request.file_count,
        request.total_size
    );

    // Auto-accept remains available for unattended devices. Otherwise ask the UI.
    let auto_accept = if let Some(storage) = crate::storage::get_storage() {
        if let Ok(Some(settings)) = storage.load_settings() {
            settings.auto_accept
        } else {
            false // Default to manual accept if settings cannot be loaded
        }
    } else {
        false // Default to manual accept if storage not initialized
    };

    let accepted = if auto_accept {
        true
    } else {
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        app_state
            .pending_transfer_requests
            .insert(request.task_id, response_tx);
        let _ = app_state.emit_async("transfer-request", &request).await;
        tokio::time::timeout(Duration::from_secs(60), response_rx)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    };

    app_state.pending_transfer_requests.remove(&request.task_id);

    let save_path = crate::storage::get_storage()
        .and_then(|storage| storage.load_settings().ok().flatten())
        .map(|settings| settings.download_path)
        .unwrap_or_else(|| get_default_download_path().to_string_lossy().to_string());

    // Create response
    let response = TransferResponse {
        task_id: request.task_id,
        accepted,
        reason: if accepted {
            None
        } else {
            Some("Transfer rejected".to_string())
        },
        save_path: Some(save_path),
    };

    // Send response
    let response_msg = ControlMessage::new(ControlMessageType::TransferResponse, &response)?;
    let response_data = response_msg.to_bytes()?;
    let len = response_data.len() as u32;

    send.write_all(&len.to_be_bytes())
        .await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
    send.write_all(&response_data)
        .await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
    send.finish()
        .await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to finish stream: {}", e)))?;

    if accepted {
        // Create receive task
        let task = TransferTask::new(
            request.task_id,
            peer_device_id,
            request.sender_name.clone(),
            TransferType::Receive,
            request.total_size,
            request.file_count,
        );

        app_state
            .transfer_tasks
            .insert(request.task_id, Arc::new(tokio::sync::Mutex::new(task)));

        let _ = recv;
    }

    Ok(())
}

/// Prepare an accepted data stream and then receive its payload.
async fn handle_data_transfer_start(
    message: ControlMessage,
    mut send: quinn::SendStream,
    recv: quinn::RecvStream,
    peer_device_id: Uuid,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    let start: DataTransferStart = message.payload()?;
    let entry = app_state
        .transfer_tasks
        .get(&start.task_id)
        .ok_or_else(|| crate::core::AppError::Other("Unknown transfer task".to_string()))?;
    let mut task = entry.lock().await;
    if task.peer_device_id != peer_device_id {
        return Err(crate::core::AppError::Other(
            "Transfer peer mismatch".to_string(),
        ));
    }
    let peer_device_name = task.peer_device_name.clone();
    task.files = start.files.clone();
    task.set_status(TransferStatus::Transferring);
    drop(task);
    drop(entry);

    let download_path = resolve_download_path();

    let missing_chunks = if let Some(storage) = crate::storage::get_storage() {
        let mut info = if start.resume {
            storage.load_resume_info(start.task_id)?
        } else {
            None
        }
        .unwrap_or_else(|| {
            crate::transfer::resume::ResumeInfo::new(
                start.task_id,
                peer_device_id,
                peer_device_name.clone(),
                TransferType::Receive,
                start.files.clone(),
                download_path.clone(),
            )
        });

        // Drop stale bitmaps if the file metadata changed since the last attempt.
        info.reconcile_files(&start.files, &download_path);
        storage.save_resume_info(&info)?;

        start
            .files
            .iter()
            .map(|file| {
                let chunk_size =
                    crate::transfer::file_chunk::FileChunker::optimal_chunk_size(file.file_size)
                        as u64;
                let total = ((file.file_size + chunk_size - 1) / chunk_size).max(1);
                let missing = (0..total)
                    .filter(|index| !info.is_chunk_received(file.file_id, *index))
                    .collect();
                (file.file_id, missing)
            })
            .collect()
    } else {
        start
            .files
            .iter()
            .map(|file| {
                let chunk_size =
                    crate::transfer::file_chunk::FileChunker::optimal_chunk_size(file.file_size)
                        as u64;
                let total = ((file.file_size + chunk_size - 1) / chunk_size).max(1);
                (file.file_id, (0..total).collect())
            })
            .collect()
    };

    let ready = DataTransferReady {
        task_id: start.task_id,
        accepted: true,
        reason: None,
        missing_chunks,
    };
    let peer_device_name_for_event = peer_device_name;
    let response = ControlMessage::new(ControlMessageType::DataTransferReady, &ready)?;
    let data = response.to_bytes()?;
    send.write_all(&(data.len() as u32).to_be_bytes())
        .await
        .map_err(|e| {
            crate::core::AppError::Network(format!("Failed to write data readiness: {}", e))
        })?;
    send.write_all(&data).await.map_err(|e| {
        crate::core::AppError::Network(format!("Failed to write data readiness payload: {}", e))
    })?;
    send.finish().await.map_err(|e| {
        crate::core::AppError::Network(format!("Failed to finish data readiness: {}", e))
    })?;

    let result = receive_files(
        recv,
        start.task_id,
        start.files,
        peer_device_id,
        app_state.clone(),
    )
    .await;
    if let Some(entry) = app_state.transfer_tasks.get(&start.task_id) {
        let mut task = entry.lock().await;
        match &result {
            Ok(_) => {
                task.set_status(TransferStatus::Completed);
                if let Some(storage) = crate::storage::get_storage() {
                    let file_names = task
                        .files
                        .iter()
                        .map(|file| file.file_name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let metadata_json = serde_json::to_string(&task.files).ok();
                    let destination_path = task
                        .save_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string());
                    let _ = storage.add_transfer_history(
                        start.task_id,
                        task.peer_device_id,
                        &task.peer_device_name,
                        "receive",
                        "completed",
                        task.total_size,
                        task.file_count,
                        Some(&file_names),
                        None,
                        destination_path.as_deref(),
                        metadata_json.as_deref(),
                    );
                }
            }
            Err(error) => {
                task.set_error(error.to_string());
                if let Some(storage) = crate::storage::get_storage() {
                    let file_names = task
                        .files
                        .iter()
                        .map(|file| file.file_name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let metadata_json = serde_json::to_string(&task.files).ok();
                    let destination_path = task
                        .save_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string());
                    let _ = storage.add_transfer_history(
                        start.task_id,
                        task.peer_device_id,
                        &task.peer_device_name,
                        "receive",
                        "failed",
                        task.total_size,
                        task.file_count,
                        Some(&file_names),
                        None,
                        destination_path.as_deref(),
                        metadata_json.as_deref(),
                    );
                }
            }
        }
    }

    match &result {
        Ok(_) => {
            let _ = app_state
                .emit_async("transfer-completed", start.task_id)
                .await;
            if let Some(handle) = app_state.get_app_handle().await {
                crate::ipc::send_notification(
                    &handle,
                    "传输完成",
                    &format!("文件已从 {} 接收完成", peer_device_name_for_event),
                );
            }
        }
        Err(error) => {
            let _ = app_state
                .emit_async(
                    "transfer-failed",
                    serde_json::json!({ "task_id": start.task_id, "error": error.to_string() }),
                )
                .await;
            if let Some(handle) = app_state.get_app_handle().await {
                crate::ipc::send_notification(
                    &handle,
                    "传输失败",
                    &format!("文件传输失败：{}", error),
                );
            }
        }
    }

    result
}

/// Get default download path
fn get_default_download_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            return PathBuf::from(format!("{}\\Downloads\\传书", userprofile));
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(format!("{}/Downloads/传书", home));
        }
    }

    PathBuf::from("./传书")
}

/// Resolve the configured download directory, expanding `~/`.
fn resolve_download_path() -> PathBuf {
    if let Some(storage) = crate::storage::get_storage() {
        if let Ok(Some(settings)) = storage.load_settings() {
            let path = settings.download_path.replace(
                "~/",
                &std::env::var("HOME").unwrap_or_else(|_| ".".to_string()),
            );
            return PathBuf::from(path);
        }
    }
    get_default_download_path()
}

/// Receive files from sender
async fn receive_files(
    mut recv: quinn::RecvStream,
    task_id: Uuid,
    files: Vec<crate::network::protocol::FileMetadata>,
    _sender_id: Uuid,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    let download_path = resolve_download_path();
    let download_limit = crate::storage::get_storage()
        .and_then(|storage| storage.load_settings().ok().flatten())
        .map(|settings| settings.download_bytes_per_sec())
        .unwrap_or(0);

    if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
        entry.lock().await.save_path = Some(download_path.clone());
    }

    // Ensure download directory exists
    std::fs::create_dir_all(&download_path)?;

    // Create rate limiter for download
    let rate_limiter = SharedRateLimiter::new(download_limit);

    let mut transferred_size: u64 = 0;
    let mut last_update_time = std::time::Instant::now();
    let mut last_transferred = 0u64;

    // Create file index for quick lookup
    let file_map: std::collections::HashMap<u64, crate::network::protocol::FileMetadata> =
        files.iter().map(|f| (f.file_id, f.clone())).collect();

    let peer_device_name = if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
        entry.lock().await.peer_device_name.clone()
    } else {
        "远程设备".to_string()
    };

    let mut resume_info = if let Some(storage) = crate::storage::get_storage() {
        storage.load_resume_info(task_id)?.unwrap_or_else(|| {
            crate::transfer::resume::ResumeInfo::new(
                task_id,
                _sender_id,
                peer_device_name.clone(),
                TransferType::Receive,
                files.clone(),
                download_path.clone(),
            )
        })
    } else {
        crate::transfer::resume::ResumeInfo::new(
            task_id,
            _sender_id,
            peer_device_name,
            TransferType::Receive,
            files.clone(),
            download_path.clone(),
        )
    };
    if let Some(storage) = crate::storage::get_storage() {
        let _ = storage.save_resume_info(&resume_info);
    }

    // Receive files
    loop {
        // Read header length
        let mut len_buf = [0u8; 4];
        match recv.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(quinn::ReadExactError::FinishedEarly) => break, // Transfer complete
            Err(e) => {
                return Err(crate::core::AppError::Network(format!(
                    "Failed to read header length: {}",
                    e
                )))
            }
        }

        let header_len = u32::from_be_bytes(len_buf) as usize;

        // Check for end of transfer marker
        if header_len == 0 {
            break;
        }

        // Read file header
        let mut header_buf = vec![0u8; header_len];
        recv.read_exact(&mut header_buf)
            .await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to read header: {}", e)))?;

        let file_header: FileHeader = bincode::deserialize(&header_buf)?;

        // Get file metadata (validated via file_header)
        let _file_meta = file_map.get(&file_header.file_id).ok_or_else(|| {
            crate::core::AppError::Other(format!(
                "File {} not found in metadata",
                file_header.file_id
            ))
        })?;

        // Update current file in task
        if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
            let mut task = entry.lock().await;
            task.set_current_file(&file_header.file_name, file_header.file_id as u32);
        }

        // Create temp file for receiving (task-scoped to avoid collisions)
        let temp_path = download_path.join(format!(".{}-{}.tmp", task_id, file_header.file_id));
        let mut temp_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&temp_path)?;

        // Pre-allocate file
        if temp_file.metadata()?.len() != file_header.file_size {
            temp_file.set_len(file_header.file_size)?;
        }

        let chunk_size =
            crate::transfer::file_chunk::FileChunker::optimal_chunk_size(file_header.file_size)
                as u64;
        let existing_chunks = resume_info
            .files
            .get(&file_header.file_id)
            .map(|file| file.chunks_received)
            .unwrap_or(0);
        let mut received_size: u64 = (existing_chunks * chunk_size).min(file_header.file_size);
        let _total_chunks = ((file_header.file_size + chunk_size - 1) / chunk_size).max(1);
        let mut chunk_index: u64 = 0;

        // Receive chunks
        while received_size < file_header.file_size {
            // Check if task is paused or canceled
            if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
                let task = entry.lock().await;
                if task.status == TransferStatus::Canceled {
                    return Err(crate::core::AppError::Other(
                        "Transfer canceled".to_string(),
                    ));
                }
                if task.status == TransferStatus::Paused {
                    drop(task);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            }

            // Read chunk packet length
            let mut chunk_len_buf = [0u8; 4];
            match recv.read_exact(&mut chunk_len_buf).await {
                Ok(_) => {}
                Err(quinn::ReadExactError::FinishedEarly) => break,
                Err(e) => {
                    return Err(crate::core::AppError::Network(format!(
                        "Failed to read chunk length: {}",
                        e
                    )))
                }
            }

            let chunk_len = u32::from_be_bytes(chunk_len_buf) as usize;

            // Check for end of file marker
            if chunk_len == 0 {
                break;
            }

            // Read chunk data
            let mut chunk_buf = vec![0u8; chunk_len];
            recv.read_exact(&mut chunk_buf).await.map_err(|e| {
                crate::core::AppError::Network(format!("Failed to read chunk: {}", e))
            })?;

            let packet: DataPacket = bincode::deserialize(&chunk_buf)?;

            // Verify checksum
            let actual_checksum = calculate_crc32(&packet.data);
            if actual_checksum != packet.metadata.checksum {
                return Err(crate::core::AppError::Other(format!(
                    "Chunk {} checksum mismatch: expected {}, got {}",
                    chunk_index, packet.metadata.checksum, actual_checksum
                )));
            }

            // Write chunk to file
            temp_file.seek(SeekFrom::Start(packet.metadata.offset))?;
            temp_file.write_all(&packet.data)?;

            received_size += packet.data.len() as u64;
            transferred_size += packet.data.len() as u64;
            chunk_index += 1;
            resume_info.mark_chunk_received(file_header.file_id, packet.metadata.chunk_index);
            resume_info.total_transferred = transferred_size;
            if chunk_index % 4 == 0 {
                if let Some(storage) = crate::storage::get_storage() {
                    let _ = storage.save_resume_info(&resume_info);
                }
            }

            // Apply rate limiting after receiving data
            if download_limit > 0 {
                rate_limiter
                    .consume_and_wait(packet.data.len() as u64)
                    .await;
            }

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

                    let info = TransferTaskInfo::from(&*task);
                    let _ = app_state.emit_async("transfer-progress", &info).await;
                }

                last_update_time = now;
                last_transferred = transferred_size;
            }
        }

        // Flush and sync file
        temp_file.flush()?;
        temp_file.sync_all()?;

        // Verify file checksum if provided
        if let Some(expected_checksum) = &file_header.checksum {
            let actual_checksum =
                crate::transfer::file_chunk::calculate_file_hash(&temp_path).await?;
            if &actual_checksum != expected_checksum {
                return Err(crate::core::AppError::Other(format!(
                    "File checksum mismatch for {}: expected {}, got {}",
                    file_header.file_name, expected_checksum, actual_checksum
                )));
            }
        }

        // Create subdirectory if relative_path is not empty (folder transfer support)
        let safe_relative_path = sanitize_relative_path(&file_header.relative_path)?;
        let final_path = if !safe_relative_path.as_os_str().is_empty() {
            let subdir = download_path.join(&safe_relative_path);
            std::fs::create_dir_all(&subdir)?;
            subdir.join(&file_header.file_name)
        } else {
            download_path.join(&file_header.file_name)
        };

        std::fs::rename(&temp_path, &final_path)?;

        log::info!(
            "Received file: {} -> {:?}",
            file_header.file_name,
            final_path
        );
    }

    if let Some(storage) = crate::storage::get_storage() {
        let _ = storage.delete_resume_info(task_id);
    }

    // Send acknowledgment
    // (connection will be closed by sender after this)

    Ok(())
}

fn sanitize_relative_path(path: &str) -> AppResult<PathBuf> {
    use std::path::Component;
    let mut safe = PathBuf::new();
    for component in std::path::Path::new(path).components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(crate::core::AppError::Other(
                    "Invalid relative file path".to_string(),
                ));
            }
        }
    }
    Ok(safe)
}

/// Handle cancel transfer request
async fn handle_cancel_transfer(
    message: ControlMessage,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    let task_id: Uuid = message.payload()?;

    if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
        let mut task = entry.lock().await;
        task.set_status(TransferStatus::Canceled);
        log::info!("Transfer task {} canceled", task_id);
    }

    // Remove temporary files and resume state; canceled transfers are not resumable.
    if let Some(storage) = crate::storage::get_storage() {
        if let Ok(Some(info)) = storage.load_resume_info(task_id) {
            for file in info.files.values() {
                let _ = std::fs::remove_file(&file.temp_path);
            }
        }
        let _ = storage.delete_resume_info(task_id);
    }

    Ok(())
}

/// Handle pause transfer request
async fn handle_pause_transfer(message: ControlMessage, app_state: Arc<AppState>) -> AppResult<()> {
    let task_id: Uuid = message.payload()?;

    if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
        let mut task = entry.lock().await;
        task.set_status(TransferStatus::Paused);
        log::info!("Transfer task {} paused", task_id);
    }

    Ok(())
}

/// Handle resume transfer request
async fn handle_resume_transfer(
    message: ControlMessage,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    let task_id: Uuid = message.payload()?;

    if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
        let mut task = entry.lock().await;
        task.set_status(TransferStatus::Transferring);
        log::info!("Transfer task {} resumed", task_id);
    }

    Ok(())
}

/// Global QUIC server instance
static QUIC_SERVER: tokio::sync::OnceCell<QuicServer> = tokio::sync::OnceCell::const_new();

/// Start the QUIC server
pub async fn start_quic_server(app_state: Arc<AppState>) -> AppResult<u16> {
    if QUIC_SERVER.get().is_none() {
        let server = QuicServer::new(QUIC_DEFAULT_PORT).await?;
        QUIC_SERVER.set(server).map_err(|_| {
            crate::core::AppError::Other("QUIC server already initialized".to_string())
        })?;
    }

    let server = QUIC_SERVER
        .get()
        .ok_or_else(|| crate::core::AppError::Other("QUIC server unavailable".to_string()))?;

    let port = server.port();
    app_state
        .set_certificate_fingerprint(server.certificate_fingerprint().to_string())
        .await;

    // Clone the endpoint handle into the accept loop so the loop owns everything
    // it needs and the global server stays alive for `stop_quic_server`.
    let endpoint = server.endpoint.clone();
    let state = app_state.clone();
    tokio::spawn(async move {
        if let Err(e) = accept_loop(endpoint, state).await {
            log::error!("QUIC server error: {}", e);
        }
    });

    log::info!("QUIC server started on port {}", port);
    Ok(port)
}

/// Accept incoming connections until the endpoint is closed.
async fn accept_loop(endpoint: quinn::Endpoint, app_state: Arc<AppState>) -> AppResult<()> {
    let pool = init_connection_pool().await;
    log::info!("QUIC server running");

    while let Some(incoming) = endpoint.accept().await {
        let app_state = app_state.clone();
        let pool = pool.clone();

        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    if let Err(e) = handle_incoming_connection(connection, app_state, pool).await {
                        log::error!("Connection handler error: {}", e);
                    }
                }
                Err(e) => {
                    log::error!("Failed to accept connection: {}", e);
                }
            }
        });
    }

    Ok(())
}

/// Stop the QUIC server
pub fn stop_quic_server() {
    if let Some(server) = QUIC_SERVER.get() {
        server.stop();
    }
}

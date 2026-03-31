use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use quinn::{Endpoint, Connection};
use tokio::time::timeout;
use uuid::Uuid;

use crate::core::{AppResult, AppState, QUIC_DEFAULT_PORT, DEFAULT_CHUNK_SIZE};
use crate::transfer::{TransferTask, TransferStatus, TransferType, TransferTaskInfo};
use crate::network::connection::{create_server_config, generate_self_signed_cert, init_connection_pool};
use crate::network::protocol::{ControlMessage, ControlMessageType, TransferRequest, TransferResponse, DataPacket};
use crate::transfer::file_chunk::calculate_crc32;
use crate::transfer::rate_limiter::SharedRateLimiter;
use crate::network::auth::authenticate_as_server;

/// QUIC server handle
pub struct QuicServer {
    endpoint: Endpoint,
    port: u16,
}

impl QuicServer {
    /// Create and bind a new QUIC server
    pub async fn new(port: u16) -> AppResult<Self> {
        // Generate self-signed certificate
        let (cert, key) = generate_self_signed_cert()?;
        let server_config = create_server_config(cert, key)?;

        // Bind to address
        let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()
            .map_err(|e| crate::core::AppError::Other(format!("Invalid address: {}", e)))?;

        let endpoint = Endpoint::server(server_config, addr)?;
        let actual_port = endpoint.local_addr()?.port();

        log::info!("QUIC server bound to port {}", actual_port);

        Ok(Self {
            endpoint,
            port: actual_port,
        })
    }

    /// Get the server port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Run the server and accept incoming connections
    pub async fn run(&self, app_state: Arc<AppState>) -> AppResult<()> {
        // Initialize connection pool
        let pool = init_connection_pool().await;

        log::info!("QUIC server running on port {}", self.port);

        while let Some(incoming) = self.endpoint.accept().await {
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
        Ok(Err(e)) => return Err(crate::core::AppError::Network(format!("Failed to accept stream: {}", e))),
        Err(_) => return Err(crate::core::AppError::Other("Connection timeout".to_string())),
    };

    // Perform server authentication
    let peer_device_id = match authenticate_as_server(
        send,
        recv,
        app_state.device_id,
        app_state.device_name.clone(),
        remote_addr,
        app_state.clone(),
    ).await {
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
    recv.read_exact(&mut len_buf).await
        .map_err(|e| crate::core::AppError::Other(format!("Failed to read message length: {}", e)))?;
    let msg_len = u32::from_be_bytes(len_buf) as usize;

    if msg_len > 10 * 1024 * 1024 { // Max 10MB message
        return Err(crate::core::AppError::Other("Message too large".to_string()));
    }

    // Read message data
    let mut msg_buf = vec![0u8; msg_len];
    recv.read_exact(&mut msg_buf).await
        .map_err(|e| crate::core::AppError::Other(format!("Failed to read message: {}", e)))?;

    // Parse control message
    let message = ControlMessage::from_bytes(&msg_buf)?;

    match message.message_type {
        ControlMessageType::TransferRequest => {
            handle_transfer_request(message, send, recv, peer_device_id, app_state).await?;
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
            log::debug!("Received unhandled message type: {:?}", message.message_type);
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

    log::info!(
        "Received transfer request from {}: {} files, {} bytes",
        request.sender_name,
        request.file_count,
        request.total_size
    );

    // Check if auto-accept is enabled in settings
    let accepted = if let Some(storage) = crate::storage::get_storage() {
        if let Ok(Some(settings)) = storage.load_settings() {
            settings.auto_accept
        } else {
            false // Default to manual accept if settings cannot be loaded
        }
    } else {
        false // Default to manual accept if storage not initialized
    };

    // Create response
    let response = TransferResponse {
        task_id: request.task_id,
        accepted,
        reason: if accepted { None } else { Some("Transfer rejected".to_string()) },
        save_path: Some("~/Downloads/传书".to_string()),
    };

    // Send response
    let response_msg = ControlMessage::new(ControlMessageType::TransferResponse, &response)?;
    let response_data = response_msg.to_bytes()?;
    let len = response_data.len() as u32;

    send.write_all(&len.to_be_bytes()).await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
    send.write_all(&response_data).await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
    send.finish().await
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

        app_state.transfer_tasks.insert(request.task_id, Arc::new(tokio::sync::Mutex::new(task)));

        // Receive files on this stream
        let task_id = request.task_id;
        let files = request.files.clone();
        let sender_id = request.sender_id;
        let sender_name = request.sender_name.clone();

        match receive_files(recv, task_id, files, sender_id, app_state.clone()).await {
            Ok(_) => {
                log::info!("Transfer {} received successfully", task_id);
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
                            "receive",
                            "completed",
                            task.total_size,
                            task.file_count,
                            Some(&file_names),
                        );
                    }

                    // Send completion notification
                    let handle = app_state.get_app_handle();
                    if let Some(h) = handle {
                        let _ = crate::ipc::emit_transfer_completed(&h, task_id, &sender_name);
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
                            "receive",
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
    }

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

/// Receive files from sender
async fn receive_files(
    mut recv: quinn::RecvStream,
    task_id: Uuid,
    files: Vec<crate::network::protocol::FileMetadata>,
    _sender_id: Uuid,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    // Get download path and limit from settings
    let (download_path, download_limit) = if let Some(storage) = crate::storage::get_storage() {
        if let Ok(Some(settings)) = storage.load_settings() {
            let path = PathBuf::from(settings.download_path.replace("~/", &std::env::var("HOME").unwrap_or_else(|_| ".".to_string())));
            (path, settings.download_limit)
        } else {
            (get_default_download_path(), 0)
        }
    } else {
        (get_default_download_path(), 0)
    };

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

    // Receive files
    loop {
        // Read header length
        let mut len_buf = [0u8; 4];
        match recv.read_exact(&mut len_buf).await {
            Ok(_) => {},
            Err(quinn::ReadExactError::FinishedEarly) => break, // Transfer complete
            Err(e) => return Err(crate::core::AppError::Network(format!("Failed to read header length: {}", e))),
        }

        let header_len = u32::from_be_bytes(len_buf) as usize;

        // Check for end of transfer marker
        if header_len == 0 {
            break;
        }

        // Read file header
        let mut header_buf = vec![0u8; header_len];
        recv.read_exact(&mut header_buf).await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to read header: {}", e)))?;

        let file_header: FileHeader = bincode::deserialize(&header_buf)?;

        // Get file metadata (validated via file_header)
        let _file_meta = file_map.get(&file_header.file_id)
            .ok_or_else(|| crate::core::AppError::Other(format!("File {} not found in metadata", file_header.file_id)))?;

        // Update current file in task
        if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
            let mut task = entry.lock().await;
            task.set_current_file(&file_header.file_name, file_header.file_id as u32);
        }

        // Create temp file for receiving
        let temp_path = download_path.join(format!(".{}.tmp", file_header.file_id));
        let mut temp_file = File::create(&temp_path)?;

        // Pre-allocate file
        temp_file.set_len(file_header.file_size)?;

        let mut received_size: u64 = 0;
        let chunk_size = DEFAULT_CHUNK_SIZE as u64;
        let _total_chunks = ((file_header.file_size + chunk_size - 1) / chunk_size).max(1);
        let mut chunk_index: u64 = 0;

        // Receive chunks
        while received_size < file_header.file_size {
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

            // Read chunk packet length
            let mut chunk_len_buf = [0u8; 4];
            match recv.read_exact(&mut chunk_len_buf).await {
                Ok(_) => {},
                Err(quinn::ReadExactError::FinishedEarly) => break,
                Err(e) => return Err(crate::core::AppError::Network(format!("Failed to read chunk length: {}", e))),
            }

            let chunk_len = u32::from_be_bytes(chunk_len_buf) as usize;

            // Check for end of file marker
            if chunk_len == 0 {
                break;
            }

            // Read chunk data
            let mut chunk_buf = vec![0u8; chunk_len];
            recv.read_exact(&mut chunk_buf).await
                .map_err(|e| crate::core::AppError::Network(format!("Failed to read chunk: {}", e)))?;

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

            // Apply rate limiting after receiving data
            if download_limit > 0 {
                rate_limiter.consume_and_wait(packet.data.len() as u64).await;
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
            let actual_checksum = crate::transfer::file_chunk::calculate_file_hash(&temp_path).await?;
            if &actual_checksum != expected_checksum {
                return Err(crate::core::AppError::Other(format!(
                    "File checksum mismatch for {}: expected {}, got {}",
                    file_header.file_name, expected_checksum, actual_checksum
                )));
            }
        }

        // Create subdirectory if relative_path is not empty (folder transfer support)
        let final_path = if !file_header.relative_path.is_empty() {
            let subdir = download_path.join(&file_header.relative_path);
            std::fs::create_dir_all(&subdir)?;
            subdir.join(&file_header.file_name)
        } else {
            download_path.join(&file_header.file_name)
        };

        std::fs::rename(&temp_path, &final_path)?;

        log::info!("Received file: {} -> {:?}", file_header.file_name, final_path);
    }

    // Send acknowledgment
    // (connection will be closed by sender after this)

    Ok(())
}

/// Handle cancel transfer request
async fn handle_cancel_transfer(
    message: ControlMessage,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    let task_id: Uuid = message.payload()?;

    if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
        let mut task = entry.lock().await;
        task.status = TransferStatus::Canceled;
        log::info!("Transfer task {} canceled", task_id);
    }

    Ok(())
}

/// Handle pause transfer request
async fn handle_pause_transfer(
    message: ControlMessage,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    let task_id: Uuid = message.payload()?;

    if let Some(entry) = app_state.transfer_tasks.get(&task_id) {
        let mut task = entry.lock().await;
        task.status = TransferStatus::Paused;
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
        task.status = TransferStatus::Transferring;
        log::info!("Transfer task {} resumed", task_id);
    }

    Ok(())
}

/// Global QUIC server instance
static QUIC_SERVER: tokio::sync::OnceCell<QuicServer> = tokio::sync::OnceCell::const_new();

/// Start the QUIC server
pub async fn start_quic_server(app_state: Arc<AppState>) -> AppResult<u16> {
    let server = QUIC_SERVER
        .get_or_init(|| async {
            QuicServer::new(QUIC_DEFAULT_PORT).await
                .expect("Failed to create QUIC server")
        })
        .await;

    let port = server.port();
    let app_state_clone = app_state.clone();

    tokio::spawn(async move {
        if let Err(e) = server.run(app_state_clone).await {
            log::error!("QUIC server error: {}", e);
        }
    });

    log::info!("QUIC server started on port {}", port);
    Ok(port)
}

/// Stop the QUIC server
pub fn stop_quic_server() {
    if let Some(server) = QUIC_SERVER.get() {
        server.stop();
    }
}

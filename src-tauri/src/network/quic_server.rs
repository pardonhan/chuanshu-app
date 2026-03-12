use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use quinn::{Endpoint, ServerConfig, Connection};
use tokio::time::timeout;
use uuid::Uuid;

use crate::core::{AppResult, AppState, QUIC_DEFAULT_PORT};
use crate::network::connection::{create_server_config, generate_self_signed_cert, get_connection_pool, init_connection_pool};
use crate::network::protocol::{ControlMessage, ControlMessageType, TransferRequest, TransferResponse};
use crate::transfer::{TransferTask, TransferStatus, TransferType};

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

    // Get device info from the connection (would need protocol handshake in real impl)
    // For now, we'll use a placeholder device ID
    let device_id = Uuid::new_v4(); // This should come from authentication

    // Add to connection pool
    let peer_conn = pool.add_incoming(device_id, connection.clone());

    // Handle streams
    loop {
        match timeout(Duration::from_secs(30), connection.accept_bi()).await {
            Ok(Ok((send, recv))) => {
                let app_state = app_state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_stream(send, recv, device_id, app_state).await {
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

    pool.remove(&device_id);
    log::info!("QUIC connection from {} closed", remote_addr);

    Ok(())
}

/// Handle a bidirectional stream
async fn handle_stream(
    mut send: quinn::SendStream,
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
            handle_transfer_request(message, send, peer_device_id, app_state).await?;
        }
        ControlMessageType::CancelTransfer => {
            handle_cancel_transfer(message, app_state).await?;
        }
        ControlMessageType::PauseTransfer => {
            handle_pause_transfer(message, app_state).await?;
        }
        ControlMessageType::ResumeTransfer => {
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

    // TODO: Check if auto-accept is enabled
    // For now, auto-accept all transfers
    let accepted = true;

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

        // Start receiving files
        // This would spawn a separate task to handle the actual file transfer
    }

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

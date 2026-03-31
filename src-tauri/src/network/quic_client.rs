use std::net::SocketAddr;
use std::sync::Arc;
use quinn::{Endpoint, Connection};
use uuid::Uuid;

use crate::core::{AppResult, AppState};
use crate::network::connection::{create_client_config, get_connection_pool, init_connection_pool};
use crate::network::protocol::{ControlMessage, ControlMessageType, TransferRequest, TransferResponse};
use crate::transfer::{TransferTask, TransferType};
use crate::network::auth::authenticate_as_client;

/// QUIC client for initiating connections
pub struct QuicClient {
    endpoint: Endpoint,
}

impl QuicClient {
    /// Create a new QUIC client
    pub fn new() -> AppResult<Self> {
        let client_config = create_client_config()?;
        let bind_addr: SocketAddr = "0.0.0.0:0".parse()
            .map_err(|e| crate::core::AppError::Other(format!("Invalid bind address: {}", e)))?;

        let mut endpoint = Endpoint::client(bind_addr)?;
        endpoint.set_default_client_config(client_config);

        Ok(Self { endpoint })
    }

    /// Get the local address
    pub fn local_addr(&self) -> AppResult<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    /// Connect to a peer and send a transfer request
    pub async fn send_transfer_request(
        &self,
        peer_addr: SocketAddr,
        peer_device_id: Uuid,
        request: TransferRequest,
        app_state: Arc<AppState>,
    ) -> AppResult<TransferResponse> {
        // Get or create connection
        let pool = init_connection_pool().await;
        let peer_conn = pool.get_or_connect(peer_device_id, peer_addr, &self.endpoint).await?;

        // Get the underlying connection for remote address
        let connection = peer_conn.connection().clone();
        let remote_addr = connection.remote_address();

        // Open stream for authentication handshake
        let (send, recv) = peer_conn.open_stream().await?;

        // Perform client authentication
        let peer_device_id = authenticate_as_client(
            send,
            recv,
            app_state.device_id,
            app_state.device_name.clone(),
            remote_addr,
            app_state.clone(),
        ).await?;

        log::info!("Authenticated with peer device {} at {}", peer_device_id, remote_addr);

        // Open new stream for transfer request after authentication
        let (mut send, mut recv) = peer_conn.open_stream().await?;

        // Send transfer request
        let message = ControlMessage::new(ControlMessageType::TransferRequest, &request)?;

        // Send message with length prefix
        let data = message.to_bytes()?;
        let len = data.len() as u32;
        send.write_all(&len.to_be_bytes()).await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
        send.write_all(&data).await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
        send.finish().await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to finish stream: {}", e)))?;

        // Read response
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await
            .map_err(|e| crate::core::AppError::Other(format!("Failed to read response length: {}", e)))?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;

        let mut resp_buf = vec![0u8; resp_len];
        recv.read_exact(&mut resp_buf).await
            .map_err(|e| crate::core::AppError::Other(format!("Failed to read response: {}", e)))?;

        let response_msg = ControlMessage::from_bytes(&resp_buf)?;
        let response: TransferResponse = response_msg.payload()?;

        // If accepted, create send task
        if response.accepted {
            let task = TransferTask::new(
                request.task_id,
                peer_device_id,
                request.sender_name.clone(),
                TransferType::Send,
                request.total_size,
                request.file_count,
            );

            app_state.transfer_tasks.insert(request.task_id, Arc::new(tokio::sync::Mutex::new(task)));
        }

        Ok(response)
    }

    /// Cancel a transfer
    pub async fn cancel_transfer(
        &self,
        _peer_addr: SocketAddr,
        peer_device_id: Uuid,
        task_id: Uuid,
    ) -> AppResult<()> {
        if let Some(pool) = get_connection_pool() {
            if let Some(peer_conn) = pool.get(&peer_device_id) {
                let message = ControlMessage::new(ControlMessageType::CancelTransfer, &task_id)?;
                peer_conn.send_control(&message).await?;
            }
        }
        Ok(())
    }

    /// Pause a transfer
    pub async fn pause_transfer(
        &self,
        _peer_addr: SocketAddr,
        peer_device_id: Uuid,
        task_id: Uuid,
    ) -> AppResult<()> {
        if let Some(pool) = get_connection_pool() {
            if let Some(peer_conn) = pool.get(&peer_device_id) {
                let message = ControlMessage::new(ControlMessageType::PauseTransfer, &task_id)?;
                peer_conn.send_control(&message).await?;
            }
        }
        Ok(())
    }

    /// Resume a transfer
    pub async fn resume_transfer(
        &self,
        _peer_addr: SocketAddr,
        peer_device_id: Uuid,
        task_id: Uuid,
    ) -> AppResult<()> {
        if let Some(pool) = get_connection_pool() {
            if let Some(peer_conn) = pool.get(&peer_device_id) {
                let message = ControlMessage::new(ControlMessageType::ResumeTransfer, &task_id)?;
                peer_conn.send_control(&message).await?;
            }
        }
        Ok(())
    }

    /// Close the client
    pub fn close(self) {
        self.endpoint.close(0u32.into(), b"client shutdown");
    }
}

/// Global QUIC client instance
static QUIC_CLIENT: tokio::sync::OnceCell<Arc<tokio::sync::Mutex<Option<QuicClient>>>> = tokio::sync::OnceCell::const_new();

/// Initialize the QUIC client
pub async fn init_quic_client() -> AppResult<()> {
    let client = QuicClient::new()?;

    QUIC_CLIENT
        .get_or_init(|| async {
            Arc::new(tokio::sync::Mutex::new(Some(client)))
        })
        .await;

    log::info!("QUIC client initialized");
    Ok(())
}

/// Get the QUIC client
pub async fn get_quic_client() -> AppResult<QuicClient> {
    let guard = QUIC_CLIENT
        .get()
        .ok_or_else(|| crate::core::AppError::Other("QUIC client not initialized".to_string()))?
        .lock().await;

    if guard.is_some() {
        // Create a new client since we can't clone the endpoint
        QuicClient::new()
    } else {
        Err(crate::core::AppError::Other("QUIC client not available".to_string()))
    }
}

/// Send a transfer request to a device
pub async fn request_transfer(
    peer_addr: SocketAddr,
    peer_device_id: Uuid,
    request: TransferRequest,
    app_state: Arc<AppState>,
) -> AppResult<TransferResponse> {
    let client = QuicClient::new()?;
    client.send_transfer_request(peer_addr, peer_device_id, request, app_state).await
}

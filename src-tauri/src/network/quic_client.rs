use quinn::Endpoint;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

use crate::core::{AppResult, AppState};
use crate::network::auth::authenticate_as_client;
use crate::network::connection::{init_connection_pool, PeerConnection};
use crate::network::protocol::{
    ControlMessage, ControlMessageType, TransferRequest, TransferResponse,
};

/// QUIC client for initiating connections.
///
/// A single endpoint is shared by the whole application so that connections
/// created by discovery and by transfers stay alive and reusable.
pub struct QuicClient {
    endpoint: Endpoint,
}

impl QuicClient {
    /// Create a new QUIC client endpoint.
    pub fn new() -> AppResult<Self> {
        let bind_addr: SocketAddr = "0.0.0.0:0"
            .parse()
            .map_err(|e| crate::core::AppError::Other(format!("Invalid bind address: {}", e)))?;
        let endpoint = Endpoint::client(bind_addr)?;
        Ok(Self { endpoint })
    }

    /// Get or create a pooled connection to a peer, pinned to its certificate.
    pub async fn connect(
        &self,
        peer_device_id: Uuid,
        peer_addr: SocketAddr,
        cert_fingerprint: Option<String>,
    ) -> AppResult<PeerConnection> {
        let pool = init_connection_pool().await;
        pool.get_or_connect(peer_device_id, peer_addr, &self.endpoint, cert_fingerprint)
            .await
    }

    /// Connect to a peer and send a transfer request.
    pub async fn send_transfer_request(
        &self,
        peer_addr: SocketAddr,
        peer_device_id: Uuid,
        request: TransferRequest,
        app_state: Arc<AppState>,
        cert_fingerprint: Option<String>,
    ) -> AppResult<(TransferResponse, PeerConnection)> {
        // Get or create connection
        let peer_conn = self
            .connect(peer_device_id, peer_addr, cert_fingerprint.clone())
            .await?;

        // Get the underlying connection for remote address
        let connection = peer_conn.connection().clone();
        let remote_addr = connection.remote_address();

        // Authenticate only the first time this pooled connection is used.
        let peer_device_id = peer_conn
            .ensure_authenticated(|send, recv| {
                authenticate_as_client(
                    send,
                    recv,
                    app_state.device_id,
                    app_state.device_name.clone(),
                    remote_addr,
                    app_state.clone(),
                    cert_fingerprint.clone(),
                )
            })
            .await?;

        log::info!(
            "Authenticated with peer device {} at {}",
            peer_device_id,
            remote_addr
        );

        // Open new stream for transfer request after authentication
        let (mut send, mut recv) = peer_conn.open_stream().await?;

        // Send transfer request
        let message = ControlMessage::new(ControlMessageType::TransferRequest, &request)?;

        // Send message with length prefix
        let data = message.to_bytes()?;
        let len = data.len() as u32;
        send.write_all(&len.to_be_bytes())
            .await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
        send.write_all(&data)
            .await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
        send.finish().await.map_err(|e| {
            crate::core::AppError::Network(format!("Failed to finish stream: {}", e))
        })?;

        // Read response
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await.map_err(|e| {
            crate::core::AppError::Other(format!("Failed to read response length: {}", e))
        })?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;

        let mut resp_buf = vec![0u8; resp_len];
        recv.read_exact(&mut resp_buf)
            .await
            .map_err(|e| crate::core::AppError::Other(format!("Failed to read response: {}", e)))?;

        let response_msg = ControlMessage::from_bytes(&resp_buf)?;
        let response: TransferResponse = response_msg.payload()?;

        Ok((response, peer_conn))
    }

    /// Close the client
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"client shutdown");
    }
}

/// Global QUIC client instance, shared across discovery and transfers.
static QUIC_CLIENT: tokio::sync::OnceCell<Arc<QuicClient>> = tokio::sync::OnceCell::const_new();

/// Initialize the QUIC client (idempotent).
pub async fn init_quic_client() -> AppResult<()> {
    get_shared_client().await.map(|_| ())
}

/// Get the shared QUIC client, creating it on first use.
pub async fn get_shared_client() -> AppResult<Arc<QuicClient>> {
    QUIC_CLIENT
        .get_or_try_init(|| async { QuicClient::new().map(Arc::new) })
        .await
        .cloned()
}

/// Send a transfer request to a device.
pub async fn request_transfer(
    peer_addr: SocketAddr,
    peer_device_id: Uuid,
    request: TransferRequest,
    app_state: Arc<AppState>,
    cert_fingerprint: Option<String>,
) -> AppResult<(TransferResponse, PeerConnection)> {
    let client = get_shared_client().await?;
    client
        .send_transfer_request(
            peer_addr,
            peer_device_id,
            request,
            app_state,
            cert_fingerprint,
        )
        .await
}

/// Close the shared client during shutdown.
pub fn close_quic_client() {
    if let Some(client) = QUIC_CLIENT.get() {
        client.close();
    }
}

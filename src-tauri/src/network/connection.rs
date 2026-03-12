use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use dashmap::DashMap;
use quinn::{Connection, Endpoint, SendStream, RecvStream};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::core::AppResult;
use crate::network::protocol::ControlMessage;

/// Connection pool for managing active QUIC connections
pub struct ConnectionPool {
    /// Active connections indexed by device ID
    connections: DashMap<Uuid, PeerConnection>,
    /// Pending connection attempts
    pending: DashMap<Uuid, tokio::sync::oneshot::Sender<AppResult<PeerConnection>>>,
}

impl ConnectionPool {
    /// Create a new connection pool
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            pending: DashMap::new(),
        }
    }

    /// Get or create a connection to a peer
    pub async fn get_or_connect(
        &self,
        device_id: Uuid,
        addr: SocketAddr,
        endpoint: &Endpoint,
    ) -> AppResult<PeerConnection> {
        // Check if we already have a connection
        if let Some(conn) = self.connections.get(&device_id) {
            if conn.is_connected().await {
                return Ok(conn.clone());
            }
            // Connection is dead, remove it
            drop(conn);
            self.connections.remove(&device_id);
        }

        // Check if there's a pending connection
        if self.pending.contains_key(&device_id) {
            // Wait a bit and retry - this is a simplified approach
            tokio::time::sleep(Duration::from_millis(100)).await;
            // Use Box::pin to avoid recursion in async
            return Box::pin(self.get_or_connect(device_id, addr, endpoint)).await;
        }

        // Create new connection
        let connection = connect_to_peer(addr, endpoint).await?;
        let peer_conn = PeerConnection::new(device_id, connection);
        self.connections.insert(device_id, peer_conn.clone());

        Ok(peer_conn)
    }

    /// Add an incoming connection
    pub fn add_incoming(&self, device_id: Uuid, connection: Connection) -> PeerConnection {
        let peer_conn = PeerConnection::new(device_id, connection);
        self.connections.insert(device_id, peer_conn.clone());
        peer_conn
    }

    /// Remove a connection
    pub fn remove(&self, device_id: &Uuid) {
        self.connections.remove(device_id);
    }

    /// Get a connection by device ID
    pub fn get(&self, device_id: &Uuid) -> Option<PeerConnection> {
        self.connections.get(device_id).map(|c| c.clone())
    }

    /// Close all connections
    pub async fn close_all(&self) {
        for entry in self.connections.iter() {
            entry.value().close().await;
        }
        self.connections.clear();
    }
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

/// A connection to a peer device
#[derive(Clone)]
pub struct PeerConnection {
    device_id: Uuid,
    connection: Connection,
    /// Active streams for this connection
    streams: Arc<Mutex<HashMap<u64, StreamHandle>>>,
    next_stream_id: Arc<Mutex<u64>>,
}

impl PeerConnection {
    /// Create a new peer connection
    pub fn new(device_id: Uuid, connection: Connection) -> Self {
        Self {
            device_id,
            connection,
            streams: Arc::new(Mutex::new(HashMap::new())),
            next_stream_id: Arc::new(Mutex::new(0)),
        }
    }

    /// Check if the connection is still active
    pub async fn is_connected(&self) -> bool {
        self.connection.close_reason().is_none()
    }

    /// Open a new bidirectional stream
    pub async fn open_stream(&self) -> AppResult<(SendStream, RecvStream)> {
        let (send, recv) = self.connection.open_bi().await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to open stream: {}", e)))?;
        Ok((send, recv))
    }

    /// Accept incoming streams
    pub async fn accept_stream(&self) -> AppResult<(SendStream, RecvStream)> {
        let (send, recv) = self.connection.accept_bi().await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to accept stream: {}", e)))?;
        Ok((send, recv))
    }

    /// Send a control message
    pub async fn send_control(&self, message: &ControlMessage) -> AppResult<()> {
        let data = message.to_bytes()?;
        let mut stream = self.connection.open_uni().await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to open stream: {}", e)))?;

        // Send length prefix
        let len = data.len() as u32;
        stream.write_all(&len.to_be_bytes()).await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
        stream.write_all(&data).await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
        stream.finish().await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to finish stream: {}", e)))?;

        Ok(())
    }

    /// Get the underlying connection
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Close the connection
    pub async fn close(&self) {
        self.connection.close(0u32.into(), b"connection closed");
    }
}

/// Handle for managing an active stream
pub struct StreamHandle {
    stream_id: u64,
    direction: StreamDirection,
    status: StreamStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamDirection {
    Outgoing,
    Incoming,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamStatus {
    Active,
    Paused,
    Closed,
}

/// Connect to a peer at the given address
async fn connect_to_peer(addr: SocketAddr, endpoint: &Endpoint) -> AppResult<Connection> {
    // Try to connect with timeout
    let connecting = endpoint.connect(addr, "chuanshu.local")
        .map_err(|e| crate::core::AppError::Other(format!("Failed to create connection: {}", e)))?;

    let connection = tokio::time::timeout(Duration::from_secs(5), connecting).await
        .map_err(|_| crate::core::AppError::Other("Connection timeout".to_string()))?
        .map_err(|e| crate::core::AppError::Other(format!("QUIC connection failed: {}", e)))?;

    Ok(connection)
}

/// Generate a self-signed certificate for QUIC
pub fn generate_self_signed_cert() -> AppResult<(rustls::Certificate, rustls::PrivateKey)> {
    use rcgen::{Certificate, DistinguishedName, DnType, PKCS_ECDSA_P256_SHA256};
    use std::net::{IpAddr, Ipv4Addr};

    let mut params = rcgen::CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, "传书文件传输");
    params.subject_alt_names = vec![
        rcgen::SanType::DnsName("chuanshu.local".to_string()),
        rcgen::SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    ];

    let cert = Certificate::from_params(params).map_err(|e|
        crate::core::AppError::Other(format!("Failed to create certificate params: {}", e))
    )?;

    let key_pair = rcgen::KeyPair::from_der(&cert.serialize_private_key_der()).map_err(|e|
        crate::core::AppError::Other(format!("Failed to extract key pair: {}", e))
    )?;

    let cert_der = cert.serialize_der().map_err(|e|
        crate::core::AppError::Other(format!("Failed to serialize certificate: {}", e))
    )?;
    let key_der = key_pair.serialize_der();

    let cert = rustls::Certificate(cert_der);
    let key = rustls::PrivateKey(key_der);

    Ok((cert, key))
}

/// Create QUIC client configuration
pub fn create_client_config() -> AppResult<quinn::ClientConfig> {
    // Create client config that accepts self-signed certificates
    // For LAN use, we configure rustls to be permissive
    let roots = rustls::RootCertStore::empty();

    // Create a permissive client config
    let crypto = std::sync::Arc::new(
        rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );

    Ok(quinn::ClientConfig::new(crypto))
}

/// Create QUIC server configuration
pub fn create_server_config(cert: rustls::Certificate, key: rustls::PrivateKey) -> AppResult<quinn::ServerConfig> {
    let crypto = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| crate::core::AppError::Other(format!("Failed to create server config: {}", e)))?;

    let crypto = std::sync::Arc::new(crypto);
    let mut config = quinn::ServerConfig::with_crypto(crypto);

    // Transport configuration for high throughput
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(100u32.into());
    transport.max_concurrent_uni_streams(100u32.into());
    transport.receive_window(16u32.into());
    transport.send_window(16 * 1024 * 1024);

    config.transport_config(Arc::new(transport));

    Ok(config)
}

/// Global connection pool
static CONNECTION_POOL: tokio::sync::OnceCell<Arc<ConnectionPool>> = tokio::sync::OnceCell::const_new();

/// Initialize the global connection pool
pub async fn init_connection_pool() -> Arc<ConnectionPool> {
    CONNECTION_POOL.get_or_init(|| async {
        Arc::new(ConnectionPool::new())
    }).await.clone()
}

/// Get the global connection pool
pub fn get_connection_pool() -> Option<Arc<ConnectionPool>> {
    CONNECTION_POOL.get().cloned()
}

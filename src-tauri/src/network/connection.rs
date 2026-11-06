use dashmap::DashMap;
use quinn::{Connection, Endpoint, RecvStream, SendStream};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::core::{AppResult, QUIC_IDLE_TIMEOUT, QUIC_KEEP_ALIVE_INTERVAL};
use crate::network::protocol::ControlMessage;

/// Verifies that the server certificate matches the fingerprint announced by discovery.
#[derive(Debug)]
struct PinnedCertVerifier {
    expected: String,
}

impl rustls::client::ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        let actual = crate::network::auth::calculate_cert_fingerprint(&end_entity.0);
        if actual != self.expected {
            log::warn!("Peer certificate fingerprint mismatch");
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

/// Connection pool for managing active QUIC connections
pub struct ConnectionPool {
    /// Active connections indexed by device ID
    connections: DashMap<Uuid, PeerConnection>,
    /// Serializes concurrent connection attempts per device
    connect_locks: DashMap<Uuid, Arc<tokio::sync::Mutex<()>>>,
    /// Connection close callbacks
    close_callbacks: DashMap<Uuid, Box<dyn Fn(Uuid) + Send + Sync>>,
}

impl ConnectionPool {
    /// Create a new connection pool
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            connect_locks: DashMap::new(),
            close_callbacks: DashMap::new(),
        }
    }

    /// Register a callback for when a connection closes
    pub fn on_connection_close<F>(&self, device_id: Uuid, callback: F)
    where
        F: Fn(Uuid) + Send + Sync + 'static,
    {
        self.close_callbacks.insert(device_id, Box::new(callback));
    }

    /// Trigger connection close callback
    pub fn trigger_close_callback(&self, device_id: Uuid) {
        if let Some((id, callback)) = self.close_callbacks.remove(&device_id) {
            callback(id);
        }
    }

    /// Get or create a connection to a peer.
    ///
    /// `cert_fingerprint` pins the peer certificate when provided. It is `None`
    /// only before the peer has ever announced its fingerprint (trust on first use).
    pub async fn get_or_connect(
        &self,
        device_id: Uuid,
        addr: SocketAddr,
        endpoint: &Endpoint,
        cert_fingerprint: Option<String>,
    ) -> AppResult<PeerConnection> {
        // Check if we already have a connection
        if let Some(conn) = self.connections.get(&device_id) {
            if conn.is_connected() {
                return Ok(conn.clone());
            }
            drop(conn);
            self.connections.remove(&device_id);
        }

        // Serialize concurrent connection attempts for the same device.
        let lock = self
            .connect_locks
            .entry(device_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        if let Some(conn) = self.connections.get(&device_id) {
            if conn.is_connected() {
                return Ok(conn.clone());
            }
            drop(conn);
            self.connections.remove(&device_id);
        }

        let connection = connect_to_peer(addr, endpoint, cert_fingerprint).await?;
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

    /// Remove a connection and trigger callback
    pub fn remove_and_callback(&self, device_id: &Uuid) {
        self.connections.remove(device_id);
        self.trigger_close_callback(*device_id);
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
    authenticated: Arc<AtomicBool>,
    authentication_lock: Arc<Mutex<()>>,
}

impl PeerConnection {
    /// Create a new peer connection
    pub fn new(device_id: Uuid, connection: Connection) -> Self {
        Self {
            device_id,
            connection,
            authenticated: Arc::new(AtomicBool::new(false)),
            authentication_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Mark an incoming connection as authenticated after the server handshake.
    pub fn mark_authenticated(&self) {
        self.authenticated.store(true, Ordering::Release);
    }

    /// Ensure the connection performs the authentication handshake at most once.
    pub async fn ensure_authenticated<F, Fut>(&self, authenticate: F) -> AppResult<Uuid>
    where
        F: FnOnce(SendStream, RecvStream) -> Fut,
        Fut: Future<Output = AppResult<Uuid>>,
    {
        if self.authenticated.load(Ordering::Acquire) {
            return Ok(self.device_id);
        }

        let _guard = self.authentication_lock.lock().await;
        if self.authenticated.load(Ordering::Acquire) {
            return Ok(self.device_id);
        }

        let (send, recv) = self.open_stream().await?;
        let authenticated_peer = authenticate(send, recv).await?;
        self.authenticated.store(true, Ordering::Release);
        Ok(authenticated_peer)
    }

    /// Start connection keep-alive and monitoring
    pub fn start_monitoring(&self, pool: Arc<ConnectionPool>) {
        let device_id = self.device_id;
        let connection = self.connection.clone();

        tokio::spawn(async move {
            // Wait for connection to close
            let closed = connection.closed();
            closed.await;

            log::info!("QUIC connection closed for device {}", device_id);

            // Remove from pool and trigger callback
            pool.remove_and_callback(&device_id);
        });
    }

    /// Check if the connection is still active
    pub fn is_connected(&self) -> bool {
        self.connection.close_reason().is_none()
    }

    /// Open a new bidirectional stream
    pub async fn open_stream(&self) -> AppResult<(SendStream, RecvStream)> {
        let (send, recv) =
            self.connection.open_bi().await.map_err(|e| {
                crate::core::AppError::Network(format!("Failed to open stream: {}", e))
            })?;
        Ok((send, recv))
    }

    /// Send a control message
    pub async fn send_control(&self, message: &ControlMessage) -> AppResult<()> {
        let data = message.to_bytes()?;
        let (mut stream, _recv) =
            self.connection.open_bi().await.map_err(|e| {
                crate::core::AppError::Network(format!("Failed to open stream: {}", e))
            })?;

        // Send length prefix
        let len = data.len() as u32;
        stream
            .write_all(&len.to_be_bytes())
            .await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
        stream
            .write_all(&data)
            .await
            .map_err(|e| crate::core::AppError::Network(format!("Failed to write: {}", e)))?;
        stream.finish().await.map_err(|e| {
            crate::core::AppError::Network(format!("Failed to finish stream: {}", e))
        })?;

        Ok(())
    }

    /// Get the underlying connection
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
}

/// Connect to a peer at the given address
async fn connect_to_peer(
    addr: SocketAddr,
    endpoint: &Endpoint,
    cert_fingerprint: Option<String>,
) -> AppResult<Connection> {
    let config = create_client_config(cert_fingerprint)?;
    let connecting = endpoint
        .connect_with(config, addr, "chuanshu.local")
        .map_err(|e| crate::core::AppError::Other(format!("Failed to create connection: {}", e)))?;

    let connection = tokio::time::timeout(Duration::from_secs(5), connecting)
        .await
        .map_err(|_| crate::core::AppError::Other("Connection timeout".to_string()))?
        .map_err(|e| crate::core::AppError::Other(format!("QUIC connection failed: {}", e)))?;

    Ok(connection)
}

/// Generate a self-signed certificate for QUIC
pub fn generate_self_signed_cert() -> AppResult<(rustls::Certificate, rustls::PrivateKey)> {
    use rcgen::{Certificate, DistinguishedName, DnType};
    use std::net::{IpAddr, Ipv4Addr};

    let mut params = rcgen::CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, "传书文件传输");
    params.subject_alt_names = vec![
        rcgen::SanType::DnsName("chuanshu.local".to_string()),
        rcgen::SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    ];

    let cert = Certificate::from_params(params).map_err(|e| {
        crate::core::AppError::Other(format!("Failed to create certificate params: {}", e))
    })?;

    let key_pair = rcgen::KeyPair::from_der(&cert.serialize_private_key_der())
        .map_err(|e| crate::core::AppError::Other(format!("Failed to extract key pair: {}", e)))?;

    let cert_der = cert.serialize_der().map_err(|e| {
        crate::core::AppError::Other(format!("Failed to serialize certificate: {}", e))
    })?;
    let key_der = key_pair.serialize_der();

    let cert = rustls::Certificate(cert_der);
    let key = rustls::PrivateKey(key_der);

    Ok((cert, key))
}

/// Create QUIC client configuration.
///
/// When `cert_fingerprint` is provided, the peer certificate is pinned to that
/// SHA-256 fingerprint. Otherwise the handshake still completes but the peer is
/// rejected as unverified (trust-on-first-use is handled by the discovery layer,
/// which always publishes the fingerprint before a transfer can start).
pub fn create_client_config(cert_fingerprint: Option<String>) -> AppResult<quinn::ClientConfig> {
    use std::sync::Arc;

    let expected = cert_fingerprint.ok_or_else(|| {
        crate::core::AppError::Network(
            "Missing peer certificate fingerprint; device has not been discovered yet".to_string(),
        )
    })?;

    let crypto = Arc::new(
        rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(Arc::new(PinnedCertVerifier { expected }))
            .with_no_client_auth(),
    );

    let mut config = quinn::ClientConfig::new(crypto);

    // Transport configuration with keep-alive
    let mut transport = quinn::TransportConfig::default();
    // Enable keep-alive: send keep-alive every QUIC_KEEP_ALIVE_INTERVAL seconds
    transport.keep_alive_interval(Some(Duration::from_secs(QUIC_KEEP_ALIVE_INTERVAL)));
    // Idle timeout: QUIC_IDLE_TIMEOUT seconds
    transport.max_idle_timeout(Some(
        Duration::from_secs(QUIC_IDLE_TIMEOUT).try_into().unwrap(),
    ));

    config.transport_config(Arc::new(transport));

    Ok(config)
}

/// Create QUIC server configuration
pub fn create_server_config(
    cert: rustls::Certificate,
    key: rustls::PrivateKey,
) -> AppResult<quinn::ServerConfig> {
    let crypto = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| {
            crate::core::AppError::Other(format!("Failed to create server config: {}", e))
        })?;

    let crypto = std::sync::Arc::new(crypto);
    let mut config = quinn::ServerConfig::with_crypto(crypto);

    // Transport configuration for high throughput and keep-alive
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(100u32.into());
    transport.max_concurrent_uni_streams(100u32.into());
    let receive_window: u32 = 16 * 1024 * 1024;
    transport.receive_window(receive_window.into());
    transport.send_window(16 * 1024 * 1024);
    // Enable keep-alive: send keep-alive every QUIC_KEEP_ALIVE_INTERVAL seconds
    transport.keep_alive_interval(Some(Duration::from_secs(QUIC_KEEP_ALIVE_INTERVAL)));
    // Idle timeout: QUIC_IDLE_TIMEOUT seconds (4x keep-alive interval)
    transport.max_idle_timeout(Some(
        Duration::from_secs(QUIC_IDLE_TIMEOUT).try_into().unwrap(),
    ));

    config.transport_config(Arc::new(transport));

    Ok(config)
}

/// Global connection pool
static CONNECTION_POOL: tokio::sync::OnceCell<Arc<ConnectionPool>> =
    tokio::sync::OnceCell::const_new();

/// Initialize the global connection pool
pub async fn init_connection_pool() -> Arc<ConnectionPool> {
    CONNECTION_POOL
        .get_or_init(|| async { Arc::new(ConnectionPool::new()) })
        .await
        .clone()
}

/// Get the global connection pool
pub fn get_connection_pool() -> Option<Arc<ConnectionPool>> {
    CONNECTION_POOL.get().cloned()
}

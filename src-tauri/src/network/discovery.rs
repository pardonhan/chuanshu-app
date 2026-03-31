use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::interval;
use uuid::Uuid;
use quinn::Endpoint;

use crate::core::{AppResult, AppError, AppState, DISCOVERY_HEARTBEAT_INTERVAL, DISCOVERY_PORT, DISCOVERY_TIMEOUT};
use crate::network::device::{Capability, DeviceInfo, OperatingSystem};
use crate::network::protocol::DiscoveryPacket;
use crate::network::connection::get_connection_pool;
use crate::storage::get_storage;

/// Event name for device list updates
const DEVICE_ONLINE_EVENT: &str = "device-online";
const DEVICE_OFFLINE_EVENT: &str = "device-offline";

/// Discovery service state
pub struct DiscoveryService {
    /// Local device ID
    device_id: Uuid,
    /// Local device name
    device_name: String,
    /// Local QUIC port
    quic_port: u16,
    /// Whether service is running
    running: Arc<Mutex<bool>>,
}

impl DiscoveryService {
    /// Create a new discovery service
    pub fn new(device_id: Uuid, device_name: String, quic_port: u16) -> Self {
        Self {
            device_id,
            device_name,
            quic_port,
            running: Arc::new(Mutex::new(false)),
        }
    }

    /// Start the discovery service
    pub async fn start(
        &self,
        app_state: Arc<AppState>,
        app_handle: AppHandle,
    ) -> AppResult<()> {
        let mut running = self.running.lock().await;
        if *running {
            return Ok(());
        }
        *running = true;
        drop(running);

        // On startup, mark all known devices as offline
        // They will be marked online as we discover them on the network
        if let Some(storage) = get_storage() {
            if let Err(e) = storage.mark_all_devices_offline() {
                log::warn!("Failed to mark all devices offline on startup: {}", e);
            } else {
                log::info!("Marked all known devices as offline for fresh start");
            }
        }

        // Create UDP socket with broadcast capability
        let socket = create_discovery_socket().await?;
        let socket_arc = Arc::new(socket);
        // Clone the socket for storage (tokio UdpSocket doesn't have try_clone, so we can't store it)
        // Instead we'll just not store it and recreate if needed

        log::info!("Discovery service started on port {}", DISCOVERY_PORT);

        // Spawn heartbeat task
        let device_id = self.device_id;
        let device_name = self.device_name.clone();
        let quic_port = self.quic_port;
        let running_clone = Arc::clone(&self.running);
        let socket_clone = Arc::clone(&socket_arc);

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(DISCOVERY_HEARTBEAT_INTERVAL));
            let os = detect_os();
            let capabilities = vec![
                Capability::FolderTransfer,
                Capability::ResumeTransfer,
                Capability::MultiDeviceSend,
            ];

            while *running_clone.lock().await {
                ticker.tick().await;

                let packet = DiscoveryPacket::announce(
                    device_id,
                    device_name.clone(),
                    os.clone(),
                    None, // IP will be determined by receiver
                    quic_port,
                    capabilities.clone(),
                );

                if let Err(e) = broadcast_discovery_packet(&socket_clone,&packet,
                ).await {
                    log::debug!("Failed to broadcast discovery: {}", e);
                }
            }
        });

        // Spawn listener task
        let device_id = self.device_id;
        let running_clone = Arc::clone(&self.running);
        let socket_clone = Arc::clone(&socket_arc);
        let app_state_clone = app_state.clone();
        let app_handle_clone = app_handle.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];

            while *running_clone.lock().await {
                match socket_clone.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        if let Err(e) = handle_discovery_packet(
                            &buf[..len],
                            addr,
                            device_id,
                            &app_state_clone,
                            &app_handle_clone,
                            &socket_clone,
                        ).await {
                            log::debug!("Failed to handle discovery packet: {}", e);
                        }
                    }
                    Err(e) => {
                        log::debug!("UDP receive error: {}", e);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        });

        // Spawn cleanup task for offline devices
        let running_clone = Arc::clone(&self.running);
        let app_state_clone = app_state.clone();
        let app_handle_clone = app_handle.clone();
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(DISCOVERY_TIMEOUT));

            while *running_clone.lock().await {
                ticker.tick().await;
                cleanup_offline_devices(&app_state_clone, &app_handle_clone).await;
            }
        });

        // Spawn known device probing task for cross-subnet discovery
        let device_id = self.device_id;
        let device_name = self.device_name.clone();
        let quic_port = self.quic_port;
        let running_clone = Arc::clone(&self.running);
        let app_state_clone = app_state.clone();
        let app_handle_clone = app_handle.clone();

        tokio::spawn(async move {
            // Wait a moment for the service to fully start, then probe periodically
            tokio::time::sleep(Duration::from_secs(2)).await;

            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            while *running_clone.lock().await {
                probe_known_devices(
                    device_id,
                    device_name.clone(),
                    quic_port,
                    Arc::clone(&running_clone),
                    app_state_clone.clone(),
                    app_handle_clone.clone()
                ).await;
                interval.tick().await;
            }
        });

        log::info!("Discovery service started with known device probing");
        Ok(())
    }

    /// Stop the discovery service
    pub async fn stop(&self) -> AppResult<()> {
        let mut running = self.running.lock().await;
        *running = false;

        // Try to send goodbye message using a temporary socket
        let os = detect_os();
        let packet = DiscoveryPacket::goodbye(
            self.device_id,
            self.device_name.clone(),
            os,
            self.quic_port,
        );

        if let Ok(data) = packet.to_bytes() {
            if let Ok(socket) = create_discovery_socket().await {
                let broadcast_addrs = get_broadcast_addresses();
                for addr in broadcast_addrs {
                    let dest = SocketAddr::new(IpAddr::V4(addr), DISCOVERY_PORT);
                    let _ = socket.send_to(&data, dest).await;
                }
            }
        }

        log::info!("Discovery service stopped");
        Ok(())
    }
}

/// Create a UDP socket for discovery
async fn create_discovery_socket() -> AppResult<tokio::net::UdpSocket> {
    // Try to bind to the discovery port with SO_REUSEADDR
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;

    // Enable address reuse
    socket.set_reuse_address(true)?;
    #[cfg(target_os = "macos")]
    socket.set_reuse_port(true)?;

    // Bind to the discovery port
    let addr: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DISCOVERY_PORT);
    socket.bind(&addr.into())?;

    // Enable broadcast
    socket.set_broadcast(true)?;

    // Set non-blocking mode
    socket.set_nonblocking(true)?;

    // Convert to tokio UdpSocket
    let std_socket: std::net::UdpSocket = socket.into();
    let tokio_socket = tokio::net::UdpSocket::from_std(std_socket)?;

    Ok(tokio_socket)
}

/// Get all local broadcast addresses
fn get_broadcast_addresses() -> Vec<Ipv4Addr> {
    let mut addresses = vec![];

    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for iface in interfaces {
            if iface.is_loopback() {
                continue;
            }

            if let IfAddr::V4(v4_addr) = iface.addr {
                if let Some(broadcast) = v4_addr.broadcast {
                    addresses.push(broadcast);
                }
            }
        }
    }

    // Fallback to global broadcast if no interfaces found
    if addresses.is_empty() {
        addresses.push(Ipv4Addr::BROADCAST);
    }

    addresses
}

use if_addrs::IfAddr;

/// Check if device was offline (not seen for > 25 seconds or doesn't exist)
fn check_device_was_offline(
    devices: &dashmap::DashMap<Uuid, DeviceInfo>,
    device_id: &Uuid,
) -> bool {
    if !devices.contains_key(device_id) {
        return true;
    }

    if let Some(device) = devices.get(device_id) {
        let elapsed = SystemTime::now().duration_since(device.value().last_seen)
            .unwrap_or(Duration::from_secs(999));
        return elapsed > Duration::from_secs(25);
    }

    true
}

/// Spawn a task to establish and monitor QUIC keep-alive connection
fn spawn_quic_connection_task(
    device_id: Uuid,
    addr: SocketAddr,
    device_info: DeviceInfo,
    _app_state: Arc<AppState>,
    app_handle: AppHandle,
) {
    tokio::spawn(async move {
        log::info!("Establishing QUIC keep-alive connection to {} ({})", device_info.device_name, addr);

        // Get connection pool
        let pool = match get_connection_pool() {
            Some(p) => p,
            None => {
                log::warn!("Connection pool not initialized");
                return;
            }
        };

        // Create QUIC client endpoint
        let client_config = match crate::network::connection::create_client_config() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Failed to create client config: {}", e);
                return;
            }
        };

        let bind_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let mut endpoint = match Endpoint::client(bind_addr) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Failed to create endpoint: {}", e);
                return;
            }
        };
        endpoint.set_default_client_config(client_config);

        // Connect to peer
        let connect_result = endpoint.connect(addr, "chuanshu.local");
        let connecting = match connect_result {
            Ok(c) => c,
            Err(e) => {
                log::debug!("Failed to initiate connection to {}: {}", addr, e);
                return;
            }
        };

        let connection = match tokio::time::timeout(Duration::from_secs(5), connecting).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                log::debug!("Failed to connect to {}: {}", addr, e);
                return;
            }
            Err(_) => {
                log::debug!("Connection timeout to {}", addr);
                return;
            }
        };

        log::info!("QUIC keep-alive connection established to {} ({})", device_info.device_name, addr);

        // Add to connection pool using public method
        let peer_conn = pool.add_incoming(device_id, connection.clone());

        // Register close callback
        let app_handle_clone = app_handle.clone();
        pool.on_connection_close(device_id, move |_id| {
            log::info!("QUIC connection closed, marking device {} as offline", _id);

            // Update database to mark as offline
            if let Some(storage) = get_storage() {
                if let Err(e) = storage.update_device_online_status(_id, false) {
                    log::warn!("Failed to update device online status: {}", e);
                }
            }

            if let Err(e) = app_handle_clone.emit("device-offline", &_id) {
                log::warn!("Failed to emit device-offline event: {}", e);
            }
        });

        // Start monitoring
        peer_conn.start_monitoring(pool.clone());

        // Wait for connection to close (the monitoring task will handle cleanup)
        connection.closed().await;
    });
}

/// Broadcast a discovery packet to all interfaces
async fn broadcast_discovery_packet(
    socket: &UdpSocket,
    packet: &DiscoveryPacket,
) -> AppResult<()> {
    let data = packet.to_bytes()?;
    let broadcast_addrs = get_broadcast_addresses();

    for addr in broadcast_addrs {
        let dest = SocketAddr::new(IpAddr::V4(addr), DISCOVERY_PORT);
        if let Err(e) = socket.send_to(&data, dest).await {
            log::debug!("Failed to send to {}: {}", dest, e);
        }
    }

    Ok(())
}

/// Handle an incoming discovery packet
async fn handle_discovery_packet(
    data: &[u8],
    addr: SocketAddr,
    local_device_id: Uuid,
    app_state: &Arc<AppState>,
    app_handle: &AppHandle,
    socket: &UdpSocket,
) -> AppResult<()> {
    let packet = DiscoveryPacket::from_bytes(data)?;

    // Ignore own packets
    if packet.device_id == local_device_id {
        return Ok(());
    }

    // Check protocol version compatibility
    if packet.protocol_version != crate::network::protocol::PROTOCOL_VERSION {
        log::debug!(
            "Ignoring device with incompatible protocol version: {} (expected {})",
            packet.protocol_version,
            crate::network::protocol::PROTOCOL_VERSION
        );
        return Ok(());
    }

    match packet.message_type {
        crate::network::protocol::DiscoveryMessageType::Announce => {
            // Update or add device
            let device_info = packet.to_device_info(addr);
            let is_new = !app_state.devices.contains_key(&packet.device_id);
            let was_offline = check_device_was_offline(&app_state.devices, &packet.device_id);

            app_state.devices.insert(packet.device_id, device_info.clone());

            // Save to known devices
            if let Some(storage) = get_storage() {
                if let Err(e) = storage.save_known_device(&device_info) {
                    log::warn!("Failed to save known device: {}", e);
                }
            }

            // Establish QUIC keep-alive connection for cross-subnet devices
            if is_new || was_offline {
                spawn_quic_connection_task(
                    packet.device_id,
                    addr,
                    device_info.clone(),
                    app_state.clone(),
                    app_handle.clone(),
                );
            }

            if is_new {
                log::info!("Device came online: {} ({})", device_info.device_name, device_info.ip_address);
                if let Err(e) = app_handle.emit(DEVICE_ONLINE_EVENT, &device_info) {
                    log::warn!("Failed to emit device-online event: {}", e);
                }
            }

            // Send response
            let response_packet = packet.response(&DeviceInfo {
                device_id: local_device_id,
                device_name: app_state.device_name.clone(),
                os: detect_os(),
                ip_address: addr.ip(),
                quic_port: app_state.devices.get(&local_device_id).map(|d| d.quic_port).unwrap_or(crate::core::QUIC_DEFAULT_PORT),
                protocol_version: crate::network::protocol::PROTOCOL_VERSION.to_string(),
                capabilities: vec![
                    Capability::FolderTransfer,
                    Capability::ResumeTransfer,
                    Capability::MultiDeviceSend,
                ],
                last_seen: SystemTime::now(),
            });

            if let Ok(response_data) = response_packet.to_bytes() {
                if let Err(e) = socket.send_to(&response_data, addr).await {
                    log::debug!("Failed to send discovery response: {}", e);
                }
            }
        }
        crate::network::protocol::DiscoveryMessageType::Response => {
            // Update device info
            let device_info = packet.to_device_info(addr);
            let is_new = !app_state.devices.contains_key(&packet.device_id);
            let was_offline = check_device_was_offline(&app_state.devices, &packet.device_id);

            app_state.devices.insert(packet.device_id, device_info.clone());

            // Save to known devices
            if let Some(storage) = get_storage() {
                if let Err(e) = storage.save_known_device(&device_info) {
                    log::warn!("Failed to save known device: {}", e);
                }
            }

            // Establish QUIC keep-alive connection for cross-subnet devices
            if is_new || was_offline {
                spawn_quic_connection_task(
                    packet.device_id,
                    addr,
                    device_info.clone(),
                    app_state.clone(),
                    app_handle.clone(),
                );
            }

            // Emit online event if it's a new device or was previously offline
            if is_new || was_offline {
                log::info!("Device discovered/updated: {} ({})", device_info.device_name, device_info.ip_address);
                if let Err(e) = app_handle.emit(DEVICE_ONLINE_EVENT, &device_info) {
                    log::warn!("Failed to emit device-online event: {}", e);
                }
            }
        }
        crate::network::protocol::DiscoveryMessageType::Goodbye => {
            // Remove device from online list
            if let Some((id, device)) = app_state.devices.remove(&packet.device_id) {
                log::info!("Device went offline: {} ({})", device.device_name, device.ip_address);

                // Mark as offline in database (but keep the record)
                if let Some(storage) = get_storage() {
                    if let Err(e) = storage.mark_device_offline(id) {
                        log::warn!("Failed to mark device as offline: {}", e);
                    }
                }

                if let Err(e) = app_handle.emit(DEVICE_OFFLINE_EVENT, &id) {
                    log::warn!("Failed to emit device-offline event: {}", e);
                }
            }
        }
    }

    Ok(())
}

/// Remove offline devices
async fn cleanup_offline_devices(app_state: &Arc<AppState>, app_handle: &AppHandle) {
    let timeout = Duration::from_secs(DISCOVERY_TIMEOUT);
    let now = SystemTime::now();
    let mut to_remove = Vec::new();

    for entry in app_state.devices.iter() {
        let device = entry.value();
        if let Ok(elapsed) = now.duration_since(device.last_seen) {
            if elapsed > timeout {
                to_remove.push(*entry.key());
            }
        }
    }

    for id in to_remove {
        if let Some((id, device)) = app_state.devices.remove(&id) {
            log::info!("Device timed out: {} ({})", device.device_name, device.ip_address);

            // Mark as offline in database (but keep the record)
            if let Some(storage) = get_storage() {
                if let Err(e) = storage.mark_device_offline(id) {
                    log::warn!("Failed to mark device as offline: {}", e);
                }
            }

            if let Err(e) = app_handle.emit(DEVICE_OFFLINE_EVENT, &id) {
                log::warn!("Failed to emit device-offline event: {}", e);
            }
        }
    }
}

/// Probe known devices from database for cross-subnet discovery
async fn probe_known_devices(
    device_id: Uuid,
    device_name: String,
    quic_port: u16,
    running: Arc<Mutex<bool>>,
    app_state: Arc<AppState>,
    app_handle: AppHandle,
) {
    use std::net::SocketAddr;
    use tokio::net::UdpSocket;

    // Get known devices from storage
    let known_devices = {
        let storage = match get_storage() {
            Some(s) => s,
            None => {
                log::debug!("Storage not available, skipping known device probe");
                return;
            }
        };

        match storage.get_known_devices() {
            Ok(devices) => devices,
            Err(e) => {
                log::debug!("Failed to get known devices: {}", e);
                return;
            }
        }
    };

    if known_devices.is_empty() {
        log::info!("No known devices to probe");
        return;
    }

    log::info!("Probing {} known devices for cross-subnet discovery", known_devices.len());

    // Create UDP socket for probing
    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            log::warn!("Failed to create probe socket: {}", e);
            return;
        }
    };

    // Create discovery packet
    let packet = DiscoveryPacket::announce(
        device_id,
        device_name,
        detect_os(),
        None,
        quic_port,
        vec![
            Capability::FolderTransfer,
            Capability::ResumeTransfer,
            Capability::MultiDeviceSend,
        ],
    );

    let data = match packet.to_bytes() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Failed to serialize probe packet: {}", e);
            return;
        }
    };

    // Probe each known device
    for known_device in known_devices {
        if !*running.lock().await {
            break;
        }

        // Parse the stored IP address
        let ip_addr = match known_device.ip_address.parse::<Ipv4Addr>() {
            Ok(ip) => ip,
            Err(e) => {
                log::debug!("Invalid IP address for device {}: {}", known_device.device_id, e);
                continue;
            }
        };

        let dest = SocketAddr::new(IpAddr::V4(ip_addr), DISCOVERY_PORT);

        log::debug!("Probing known device at {}: {}", known_device.device_name, ip_addr);

        // Send probe packet
        if let Err(e) = socket.send_to(&data, dest).await {
            log::debug!("Failed to send probe to {}: {}", dest, e);
            continue;
        }

        // Brief pause to avoid overwhelming the network
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Listen for responses (with timeout)
    let mut buf = vec![0u8; 65535];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    while *running.lock().await && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut buf)).await {
            Ok(Ok((len, addr))) => {
                if let Err(e) = handle_discovery_packet(
                    &buf[..len],
                    addr,
                    device_id,
                    &app_state,
                    &app_handle,
                    &socket,
                ).await {
                    log::debug!("Failed to handle probe response: {}", e);
                }
            }
            Ok(Err(e)) => {
                log::debug!("Probe receive error: {}", e);
            }
            Err(_) => {
                // Timeout, continue loop
            }
        }
    }

    log::info!("Known device probing completed");
}
fn detect_os() -> OperatingSystem {
    #[cfg(target_os = "windows")]
    return OperatingSystem::Windows;
    #[cfg(target_os = "macos")]
    return OperatingSystem::MacOS;
    #[cfg(target_os = "linux")]
    return OperatingSystem::Linux;
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    return OperatingSystem::Unknown;
}

/// Global discovery service instance
static DISCOVERY_SERVICE: tokio::sync::OnceCell<DiscoveryService> = tokio::sync::OnceCell::const_new();

/// Start the discovery service
pub async fn start_discovery_service(
    app_state: Arc<AppState>,
    app_handle: AppHandle,
) -> AppResult<()> {
    let service = DISCOVERY_SERVICE
        .get_or_init(|| async {
            DiscoveryService::new(
                app_state.device_id,
                app_state.device_name.clone(),
                crate::core::QUIC_DEFAULT_PORT,
            )
        })
        .await;

    service.start(app_state, app_handle).await
}

/// Stop the discovery service
pub async fn stop_discovery_service() -> AppResult<()> {
    if let Some(service) = DISCOVERY_SERVICE.get() {
        service.stop().await?;
    }
    Ok(())
}

/// Discover a device by IP address
#[tauri::command]
pub async fn discover_device_by_ip(
    ip: String,
    state: tauri::State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
) -> AppResult<Option<DeviceInfo>> {
    use std::net::SocketAddr;
    use tokio::net::UdpSocket;

    // Parse the IP address
    let ip_addr = ip.parse::<Ipv4Addr>()
        .map_err(|e| AppError::Config(format!("Invalid IP address: {}", e)))?;

    // Create UDP socket
    let socket = UdpSocket::bind("0.0.0.0:0").await?;

    // Send discovery packet
    let dest = SocketAddr::new(IpAddr::V4(ip_addr), DISCOVERY_PORT);

    let packet = DiscoveryPacket::announce(
        state.device_id,
        state.device_name.clone(),
        detect_os(),
        None,
        crate::core::QUIC_DEFAULT_PORT,
        vec![
            Capability::FolderTransfer,
            Capability::ResumeTransfer,
            Capability::MultiDeviceSend,
        ],
    );

    let data = packet.to_bytes()?;
    socket.send_to(&data, dest).await?;

    // Wait for response with timeout
    let mut buf = vec![0u8; 65535];

    match tokio::time::timeout(
        Duration::from_secs(3),
        socket.recv_from(&mut buf)
    ).await {
        Ok(Ok((len, addr))) => {
            if let Ok(packet) = DiscoveryPacket::from_bytes(&buf[..len]) {
                if packet.message_type == crate::network::protocol::DiscoveryMessageType::Response
                    && packet.protocol_version == crate::network::protocol::PROTOCOL_VERSION {

                    let device_info = packet.to_device_info(addr);

                    // Add to device list
                    state.devices.insert(packet.device_id, device_info.clone());

                    // Save to known devices
                    if let Some(storage) = get_storage() {
                        if let Err(e) = storage.save_known_device(&device_info) {
                            log::warn!("Failed to save known device: {}", e);
                        }
                        if let Err(e) = storage.update_last_connected(packet.device_id) {
                            log::warn!("Failed to update last connected: {}", e);
                        }
                    }

                    if let Err(e) = app_handle.emit(DEVICE_ONLINE_EVENT, &device_info) {
                        log::warn!("Failed to emit device-online event: {}", e);
                    }

                    log::info!("Device discovered at {}: {} ({})", ip, device_info.device_name, device_info.device_id);
                    return Ok(Some(device_info));
                }
            }
        }
        Ok(Err(e)) => {
            log::debug!("UDP receive error: {}", e);
        }
        Err(_) => {
            log::debug!("Discovery timeout for IP: {}", ip);
        }
    }

    Ok(None)
}

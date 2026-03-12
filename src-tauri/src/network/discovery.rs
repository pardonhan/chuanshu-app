use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::interval;
use uuid::Uuid;

use crate::core::{AppResult, AppState, DISCOVERY_HEARTBEAT_INTERVAL, DISCOVERY_PORT, DISCOVERY_TIMEOUT};
use crate::network::device::{Capability, DeviceInfo, OperatingSystem};
use crate::network::protocol::DiscoveryPacket;

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
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(DISCOVERY_TIMEOUT));

            while *running_clone.lock().await {
                ticker.tick().await;
                cleanup_offline_devices(&app_state, &app_handle).await;
            }
        });

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

            app_state.devices.insert(packet.device_id, device_info.clone());

            if is_new {
                log::info!("Device came online: {} ({})", device_info.device_name, device_info.ip_address);
                let _ = app_handle.emit(DEVICE_ONLINE_EVENT, &device_info);
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
                let _ = socket.send_to(&response_data, addr).await;
            }
        }
        crate::network::protocol::DiscoveryMessageType::Response => {
            // Update device info
            let device_info = packet.to_device_info(addr);
            let is_new = !app_state.devices.contains_key(&packet.device_id);

            app_state.devices.insert(packet.device_id, device_info.clone());

            if is_new {
                log::info!("Device discovered: {} ({})", device_info.device_name, device_info.ip_address);
                let _ = app_handle.emit(DEVICE_ONLINE_EVENT, &device_info);
            }
        }
        crate::network::protocol::DiscoveryMessageType::Goodbye => {
            // Remove device
            if let Some((id, device)) = app_state.devices.remove(&packet.device_id) {
                log::info!("Device went offline: {} ({})", device.device_name, device.ip_address);
                let _ = app_handle.emit(DEVICE_OFFLINE_EVENT, &id);
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
            let _ = app_handle.emit(DEVICE_OFFLINE_EVENT, &id);
        }
    }
}

/// Detect the current operating system
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

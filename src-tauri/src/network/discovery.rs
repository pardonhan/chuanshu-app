use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::net::UdpSocket;
use tokio::time::interval;
use uuid::Uuid;

use crate::core::{AppError, AppResult, AppState, DISCOVERY_HEARTBEAT_INTERVAL, DISCOVERY_PORT, DISCOVERY_TIMEOUT, QUIC_DEFAULT_PORT};
use crate::network::device::{now_secs, Capability, DeviceInfo, OperatingSystem};
use crate::network::protocol::DiscoveryPacket;
use crate::storage::get_storage;

/// Event name for device list updates
const DEVICE_ONLINE_EVENT: &str = "device-online";
const DEVICE_OFFLINE_EVENT: &str = "device-offline";

/// 已知设备写库节流（秒）
const KNOWN_DEVICE_PERSIST_INTERVAL: u64 = 30;

/// Discovery service state
pub struct DiscoveryService {
    app_state: Arc<AppState>,
    running: Arc<AtomicBool>,
}

impl DiscoveryService {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self {
            app_state,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start the discovery service
    pub async fn start(&self, app_handle: AppHandle) -> AppResult<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        // On startup, mark all known devices as offline
        if let Some(storage) = get_storage() {
            if let Err(e) = storage.mark_all_devices_offline() {
                log::warn!("Failed to mark all devices offline on startup: {}", e);
            }
        }

        let socket = create_discovery_socket().await?;
        let socket = Arc::new(socket);
        let quic_port = QUIC_DEFAULT_PORT;

        log::info!("Discovery service started on port {}", DISCOVERY_PORT);

        // 心跳广播
        {
            let app_state = self.app_state.clone();
            let running = self.running.clone();
            let socket = socket.clone();
            tokio::spawn(async move {
                let mut ticker = interval(Duration::from_secs(DISCOVERY_HEARTBEAT_INTERVAL));
                let os = detect_os();
                while running.load(Ordering::Relaxed) {
                    ticker.tick().await;
                    let packet = DiscoveryPacket::announce(
                        app_state.device_id,
                        app_state.device_name(),
                        os.clone(),
                        None,
                        quic_port,
                        capabilities(),
                    );
                    if let Err(e) = broadcast_discovery_packet(&socket, &packet).await {
                        log::debug!("Failed to broadcast discovery: {}", e);
                    }
                }
            });
        }

        // 监听
        {
            let app_state = self.app_state.clone();
            let running = self.running.clone();
            let socket = socket.clone();
            let app_handle = app_handle.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65535];
                while running.load(Ordering::Relaxed) {
                    match socket.recv_from(&mut buf).await {
                        Ok((len, addr)) => {
                            if let Err(e) = handle_discovery_packet(
                                &buf[..len],
                                addr,
                                &app_state,
                                &app_handle,
                                &socket,
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
        }

        // 离线清理
        {
            let app_state = self.app_state.clone();
            let running = self.running.clone();
            let app_handle = app_handle.clone();
            tokio::spawn(async move {
                let mut ticker = interval(Duration::from_secs(DISCOVERY_TIMEOUT));
                while running.load(Ordering::Relaxed) {
                    ticker.tick().await;
                    cleanup_offline_devices(&app_state, &app_handle).await;
                }
            });
        }

        // 已知设备探测（跨网段）
        {
            let app_state = self.app_state.clone();
            let running = self.running.clone();
            let app_handle = app_handle.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let mut ticker = tokio::time::interval(Duration::from_secs(10));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                while running.load(Ordering::Relaxed) {
                    probe_known_devices(quic_port, running.clone(), app_state.clone(), app_handle.clone()).await;
                    ticker.tick().await;
                }
            });
        }

        Ok(())
    }

    /// Stop the discovery service
    pub async fn stop(&self) -> AppResult<()> {
        self.running.store(false, Ordering::SeqCst);

        let packet = DiscoveryPacket::goodbye(
            self.app_state.device_id,
            self.app_state.device_name(),
            detect_os(),
            QUIC_DEFAULT_PORT,
        );

        if let Ok(data) = packet.to_bytes() {
            if let Ok(socket) = create_discovery_socket().await {
                for addr in get_broadcast_addresses() {
                    let dest = SocketAddr::new(IpAddr::V4(addr), DISCOVERY_PORT);
                    let _ = socket.send_to(&data, dest).await;
                }
            }
        }

        log::info!("Discovery service stopped");
        Ok(())
    }
}

fn capabilities() -> Vec<Capability> {
    vec![Capability::FolderTransfer, Capability::MultiDeviceSend]
}

/// Create a UDP socket for discovery
async fn create_discovery_socket() -> AppResult<tokio::net::UdpSocket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;

    socket.set_reuse_address(true)?;
    #[cfg(target_os = "macos")]
    socket.set_reuse_port(true)?;

    let addr: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DISCOVERY_PORT);
    socket.bind(&addr.into())?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;

    let std_socket: std::net::UdpSocket = socket.into();
    Ok(tokio::net::UdpSocket::from_std(std_socket)?)
}

/// Get all local broadcast addresses
fn get_broadcast_addresses() -> Vec<Ipv4Addr> {
    use if_addrs::IfAddr;

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

    if addresses.is_empty() {
        addresses.push(Ipv4Addr::BROADCAST);
    }

    addresses
}

/// Check if device was offline (not seen for > 25 seconds or doesn't exist)
fn check_device_was_offline(devices: &dashmap::DashMap<Uuid, DeviceInfo>, device_id: &Uuid) -> bool {
    match devices.get(device_id) {
        Some(device) => now_secs().saturating_sub(device.last_seen) > 25,
        None => true,
    }
}

/// Broadcast a discovery packet to all interfaces
async fn broadcast_discovery_packet(socket: &UdpSocket, packet: &DiscoveryPacket) -> AppResult<()> {
    let data = packet.to_bytes()?;

    for addr in get_broadcast_addresses() {
        let dest = SocketAddr::new(IpAddr::V4(addr), DISCOVERY_PORT);
        if let Err(e) = socket.send_to(&data, dest).await {
            log::debug!("Failed to send to {}: {}", dest, e);
        }
    }

    Ok(())
}

/// 记录设备并（节流）持久化
fn record_device(
    app_state: &Arc<AppState>,
    device_info: &DeviceInfo,
) {
    let should_persist = match app_state.devices.get(&device_info.device_id) {
        Some(prev) => {
            prev.device_name != device_info.device_name
                || prev.ip_address != device_info.ip_address
                || prev.quic_port != device_info.quic_port
                || now_secs().saturating_sub(prev.last_seen) >= KNOWN_DEVICE_PERSIST_INTERVAL
        }
        None => true,
    };

    app_state.devices.insert(device_info.device_id, device_info.clone());

    if should_persist {
        if let Some(storage) = get_storage() {
            if let Err(e) = storage.save_known_device(device_info) {
                log::warn!("Failed to save known device: {}", e);
            }
        }
    }
}

/// Handle an incoming discovery packet
async fn handle_discovery_packet(
    data: &[u8],
    addr: SocketAddr,
    app_state: &Arc<AppState>,
    app_handle: &AppHandle,
    socket: &UdpSocket,
) -> AppResult<()> {
    let packet = DiscoveryPacket::from_bytes(data)?;

    if packet.device_id == app_state.device_id {
        return Ok(());
    }

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
            let device_info = packet.to_device_info(addr);
            let is_new = !app_state.devices.contains_key(&packet.device_id);
            let was_offline = check_device_was_offline(&app_state.devices, &packet.device_id);

            record_device(app_state, &device_info);

            if is_new || was_offline {
                log::info!("Device online: {} ({})", device_info.device_name, device_info.ip_address);
                let _ = app_handle.emit(DEVICE_ONLINE_EVENT, &device_info);
            }

            let response_packet = packet.response(&DeviceInfo {
                device_id: app_state.device_id,
                device_name: app_state.device_name(),
                os: detect_os(),
                ip_address: addr.ip(),
                quic_port: QUIC_DEFAULT_PORT,
                protocol_version: crate::network::protocol::PROTOCOL_VERSION.to_string(),
                capabilities: capabilities(),
                last_seen: now_secs(),
            });

            if let Ok(response_data) = response_packet.to_bytes() {
                if let Err(e) = socket.send_to(&response_data, addr).await {
                    log::debug!("Failed to send discovery response: {}", e);
                }
            }
        }
        crate::network::protocol::DiscoveryMessageType::Response => {
            let device_info = packet.to_device_info(addr);
            let is_new = !app_state.devices.contains_key(&packet.device_id);
            let was_offline = check_device_was_offline(&app_state.devices, &packet.device_id);

            record_device(app_state, &device_info);

            if is_new || was_offline {
                log::info!("Device discovered/updated: {} ({})", device_info.device_name, device_info.ip_address);
                let _ = app_handle.emit(DEVICE_ONLINE_EVENT, &device_info);
            }
        }
        crate::network::protocol::DiscoveryMessageType::Goodbye => {
            if let Some((id, device)) = app_state.devices.remove(&packet.device_id) {
                log::info!("Device went offline: {} ({})", device.device_name, device.ip_address);

                if let Some(storage) = get_storage() {
                    if let Err(e) = storage.mark_device_offline(id) {
                        log::warn!("Failed to mark device as offline: {}", e);
                    }
                }

                let _ = app_handle.emit(DEVICE_OFFLINE_EVENT, &id);
            }
        }
    }

    Ok(())
}

/// Remove offline devices
async fn cleanup_offline_devices(app_state: &Arc<AppState>, app_handle: &AppHandle) {
    let now = now_secs();
    let mut to_remove = Vec::new();

    for entry in app_state.devices.iter() {
        if now.saturating_sub(entry.value().last_seen) > DISCOVERY_TIMEOUT {
            to_remove.push(*entry.key());
        }
    }

    for id in to_remove {
        if let Some((id, device)) = app_state.devices.remove(&id) {
            log::info!("Device timed out: {} ({})", device.device_name, device.ip_address);

            if let Some(storage) = get_storage() {
                if let Err(e) = storage.mark_device_offline(id) {
                    log::warn!("Failed to mark device as offline: {}", e);
                }
            }

            let _ = app_handle.emit(DEVICE_OFFLINE_EVENT, &id);
        }
    }
}

/// Probe known devices from database for cross-subnet discovery
async fn probe_known_devices(
    quic_port: u16,
    running: Arc<AtomicBool>,
    app_state: Arc<AppState>,
    app_handle: AppHandle,
) {
    let known_devices = match get_storage() {
        Some(storage) => match storage.get_known_devices() {
            Ok(devices) => devices,
            Err(e) => {
                log::debug!("Failed to get known devices: {}", e);
                return;
            }
        },
        None => return,
    };

    if known_devices.is_empty() {
        return;
    }

    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            log::warn!("Failed to create probe socket: {}", e);
            return;
        }
    };

    let packet = DiscoveryPacket::announce(
        app_state.device_id,
        app_state.device_name(),
        detect_os(),
        None,
        quic_port,
        capabilities(),
    );

    let data = match packet.to_bytes() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Failed to serialize probe packet: {}", e);
            return;
        }
    };

    for known_device in known_devices {
        if !running.load(Ordering::Relaxed) {
            break;
        }

        let ip_addr = match known_device.ip_address.parse::<Ipv4Addr>() {
            Ok(ip) => ip,
            Err(_) => continue,
        };

        let dest = SocketAddr::new(IpAddr::V4(ip_addr), DISCOVERY_PORT);
        if let Err(e) = socket.send_to(&data, dest).await {
            log::debug!("Failed to send probe to {}: {}", dest, e);
            continue;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let mut buf = vec![0u8; 65535];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);

    while running.load(Ordering::Relaxed) && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut buf)).await {
            Ok(Ok((len, addr))) => {
                if let Err(e) = handle_discovery_packet(
                    &buf[..len],
                    addr,
                    &app_state,
                    &app_handle,
                    &socket,
                ).await {
                    log::debug!("Failed to handle probe response: {}", e);
                }
            }
            Ok(Err(e)) => log::debug!("Probe receive error: {}", e),
            Err(_) => {}
        }
    }
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
        .get_or_init(|| async { DiscoveryService::new(app_state) })
        .await;

    service.start(app_handle).await
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
    let ip_addr = ip
        .parse::<Ipv4Addr>()
        .map_err(|e| AppError::Config(format!("Invalid IP address: {}", e)))?;

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let dest = SocketAddr::new(IpAddr::V4(ip_addr), DISCOVERY_PORT);

    let packet = DiscoveryPacket::announce(
        state.device_id,
        state.device_name(),
        detect_os(),
        None,
        QUIC_DEFAULT_PORT,
        capabilities(),
    );

    let data = packet.to_bytes()?;
    socket.send_to(&data, dest).await?;

    let mut buf = vec![0u8; 65535];

    match tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut buf)).await {
        Ok(Ok((len, addr))) => {
            if let Ok(packet) = DiscoveryPacket::from_bytes(&buf[..len]) {
                if packet.message_type == crate::network::protocol::DiscoveryMessageType::Response
                    && packet.protocol_version == crate::network::protocol::PROTOCOL_VERSION
                {
                    let device_info = packet.to_device_info(addr);

                    state.devices.insert(packet.device_id, device_info.clone());

                    if let Some(storage) = get_storage() {
                        if let Err(e) = storage.save_known_device(&device_info) {
                            log::warn!("Failed to save known device: {}", e);
                        }
                        if let Err(e) = storage.update_last_connected(packet.device_id) {
                            log::warn!("Failed to update last connected: {}", e);
                        }
                    }

                    let _ = app_handle.emit(DEVICE_ONLINE_EVENT, &device_info);

                    log::info!("Device discovered at {}: {} ({})", ip, device_info.device_name, device_info.device_id);
                    return Ok(Some(device_info));
                }
            }
        }
        Ok(Err(e)) => log::debug!("UDP receive error: {}", e),
        Err(_) => log::debug!("Discovery timeout for IP: {}", ip),
    }

    Ok(None)
}

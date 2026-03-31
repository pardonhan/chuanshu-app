/// Authentication and handshake protocol for QUIC connections
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

use crate::core::{AppResult, AppState};
use crate::network::protocol::{
    AuthHello, AuthResponse, AuthAck, ControlMessage, ControlMessageType,
};

/// Perform authentication handshake as client (initiator)
pub async fn authenticate_as_client(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    local_device_id: Uuid,
    local_device_name: String,
    remote_addr: SocketAddr,
    app_state: Arc<AppState>,
) -> AppResult<Uuid> {
    use sha2::{Sha256, Digest};

    log::info!("Starting client authentication handshake");

    // Generate random nonce
    let nonce: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();

    // Send AuthHello
    let hello = AuthHello {
        device_id: local_device_id,
        device_name: local_device_name.clone(),
        protocol_version: crate::network::protocol::PROTOCOL_VERSION.to_string(),
        nonce: nonce.clone(),
    };

    let hello_msg = ControlMessage::new(ControlMessageType::AuthHello, &hello)?;
    send_control_message(&mut send, &hello_msg).await?;

    // Receive AuthResponse
    let response_msg = recv_control_message(&mut recv).await?;
    let response: AuthResponse = response_msg.payload()?;

    log::info!("Received auth response from device {} ({})", response.device_id, response.device_name);

    // Verify the peer's signature (simplified - in production, use proper digital signatures)
    let expected_hash = Sha256::digest(&[&nonce[..], response.device_id.as_bytes()].concat());
    let expected_signature = expected_hash.to_vec();

    // For now, accept any signature (simplified for LAN use)
    // In production, verify using peer's public key
    let _ = expected_signature;

    // Store peer device info
    app_state.devices.insert(
        response.device_id,
        crate::network::device::DeviceInfo {
            device_id: response.device_id,
            device_name: response.device_name.clone(),
            os: crate::network::device::OperatingSystem::Unknown,
            ip_address: remote_addr.ip(),
            quic_port: remote_addr.port(),
            protocol_version: hello.protocol_version.clone(),
            capabilities: vec![],
            last_seen: std::time::SystemTime::now(),
        }
    );

    // Send AuthAck
    let ack = AuthAck {
        device_id: local_device_id,
        signature: vec![], // Simplified - would sign response.nonce in production
        authenticated: true,
        error: None,
    };

    let ack_msg = ControlMessage::new(ControlMessageType::AuthAck, &ack)?;
    send_control_message(&mut send, &ack_msg).await?;

    log::info!("Client authentication completed with device {}", response.device_id);

    Ok(response.device_id)
}

/// Perform authentication handshake as server (responder)
pub async fn authenticate_as_server(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    local_device_id: Uuid,
    local_device_name: String,
    remote_addr: SocketAddr,
    app_state: Arc<AppState>,
) -> AppResult<Uuid> {
    use sha2::{Sha256, Digest};

    log::info!("Starting server authentication handshake");

    // Receive AuthHello
    let hello_msg = recv_control_message(&mut recv).await?;
    let hello: AuthHello = hello_msg.payload()?;

    log::info!("Received auth hello from device {} ({})", hello.device_id, hello.device_name);

    // Generate random nonce for response
    let response_nonce: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();

    // Create signature (simplified - in production, sign with private key)
    let hash = Sha256::digest(&[&hello.nonce[..], local_device_id.as_bytes()].concat());
    let signature = hash.to_vec();

    // Get certificate fingerprint (simplified - would use actual cert in production)
    let cert_fingerprint = format!("mock-fingerprint-{}", local_device_id);

    // Send AuthResponse
    let response = AuthResponse {
        device_id: local_device_id,
        device_name: local_device_name.clone(),
        cert_fingerprint,
        signature,
        nonce: response_nonce.clone(),
    };

    let response_msg = ControlMessage::new(ControlMessageType::AuthResponse, &response)?;
    send_control_message(&mut send, &response_msg).await?;

    // Receive AuthAck
    let ack_msg = recv_control_message(&mut recv).await?;
    let ack: AuthAck = ack_msg.payload()?;

    if !ack.authenticated {
        return Err(crate::core::AppError::Other(
            format!("Authentication failed: {}", ack.error.unwrap_or_default())
        ));
    }

    // Store peer device info
    app_state.devices.insert(
        hello.device_id,
        crate::network::device::DeviceInfo {
            device_id: hello.device_id,
            device_name: hello.device_name.clone(),
            os: crate::network::device::OperatingSystem::Unknown,
            ip_address: remote_addr.ip(),
            quic_port: remote_addr.port(),
            protocol_version: hello.protocol_version.clone(),
            capabilities: vec![],
            last_seen: std::time::SystemTime::now(),
        }
    );

    log::info!("Server authentication completed with device {}", hello.device_id);

    Ok(hello.device_id)
}

/// Send control message with length prefix
async fn send_control_message(
    send: &mut quinn::SendStream,
    message: &ControlMessage,
) -> AppResult<()> {
    let data = message.to_bytes()?;
    let len = data.len() as u32;
    send.write_all(&len.to_be_bytes()).await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write message: {}", e)))?;
    send.write_all(&data).await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write message: {}", e)))?;
    Ok(())
}

/// Receive control message with length prefix
async fn recv_control_message(
    recv: &mut quinn::RecvStream,
) -> AppResult<ControlMessage> {
    use tokio::io::AsyncReadExt;

    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to read message length: {}", e)))?;
    let msg_len = u32::from_be_bytes(len_buf) as usize;

    if msg_len > 10 * 1024 * 1024 {
        return Err(crate::core::AppError::Other("Message too large".to_string()));
    }

    let mut msg_buf = vec![0u8; msg_len];
    recv.read_exact(&mut msg_buf).await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to read message: {}", e)))?;

    ControlMessage::from_bytes(&msg_buf)
}

/// Generate a random nonce
pub fn generate_nonce() -> Vec<u8> {
    (0..32).map(|_| rand::random::<u8>()).collect()
}

/// Calculate certificate fingerprint
pub fn calculate_cert_fingerprint(cert_der: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(cert_der);
    format!("{:x}", hash)
}

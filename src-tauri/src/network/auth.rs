/// Authentication and handshake protocol for QUIC connections
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

use crate::core::{AppResult, AppState};
use crate::network::device::{DeviceInfo, OperatingSystem};
use crate::network::protocol::{
    AuthAck, AuthHello, AuthResponse, ControlMessage, ControlMessageType, PROTOCOL_VERSION,
};

/// Perform authentication handshake as client (initiator).
///
/// `expected_cert_fingerprint` is the fingerprint advertised through discovery.
/// When present, the peer's claimed fingerprint must match it.
#[allow(clippy::too_many_arguments)]
pub async fn authenticate_as_client(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    local_device_id: Uuid,
    local_device_name: String,
    remote_addr: SocketAddr,
    app_state: Arc<AppState>,
    expected_cert_fingerprint: Option<String>,
) -> AppResult<Uuid> {
    use sha2::{Digest, Sha256};

    log::info!("Starting client authentication handshake");

    let nonce: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
    let local_fingerprint = app_state
        .certificate_fingerprint()
        .await
        .unwrap_or_default();

    let hello = AuthHello {
        device_id: local_device_id,
        device_name: local_device_name.clone(),
        protocol_version: PROTOCOL_VERSION.to_string(),
        cert_fingerprint: local_fingerprint,
        nonce: nonce.clone(),
    };

    let hello_msg = ControlMessage::new(ControlMessageType::AuthHello, &hello)?;
    send_control_message(&mut send, &hello_msg).await?;

    let response_msg = recv_control_message(&mut recv).await?;
    let response: AuthResponse = response_msg.payload()?;

    log::info!(
        "Received auth response from device {} ({})",
        response.device_id,
        response.device_name
    );

    // Verify the peer's challenge response before accepting the connection.
    let expected_hash = Sha256::digest(&[&nonce[..], response.device_id.as_bytes()].concat());
    if response.signature != expected_hash.to_vec() {
        return Err(crate::core::AppError::Other(
            "Peer authentication signature mismatch".to_string(),
        ));
    }
    if response.cert_fingerprint.is_empty() || response.cert_fingerprint.starts_with("mock-") {
        return Err(crate::core::AppError::Other(
            "Peer certificate fingerprint is invalid".to_string(),
        ));
    }
    if let Some(expected) = &expected_cert_fingerprint {
        if &response.cert_fingerprint != expected {
            return Err(crate::core::AppError::Other(
                "Peer certificate fingerprint does not match discovery record".to_string(),
            ));
        }
    }

    upsert_device_info(
        &app_state,
        DeviceInfo {
            device_id: response.device_id,
            device_name: response.device_name.clone(),
            os: OperatingSystem::Unknown,
            ip_address: remote_addr.ip(),
            quic_port: remote_addr.port(),
            protocol_version: hello.protocol_version.clone(),
            capabilities: vec![],
            last_seen: std::time::SystemTime::now(),
            cert_fingerprint: response.cert_fingerprint.clone(),
        },
    );

    let ack = AuthAck {
        device_id: local_device_id,
        signature: Sha256::digest(&[&response.nonce[..], local_device_id.as_bytes()].concat())
            .to_vec(),
        authenticated: true,
        error: None,
    };

    let ack_msg = ControlMessage::new(ControlMessageType::AuthAck, &ack)?;
    send_control_message(&mut send, &ack_msg).await?;

    log::info!(
        "Client authentication completed with device {}",
        response.device_id
    );

    Ok(response.device_id)
}

/// Perform authentication handshake as server (responder).
#[allow(clippy::too_many_arguments)]
pub async fn authenticate_as_server(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    local_device_id: Uuid,
    local_device_name: String,
    remote_addr: SocketAddr,
    app_state: Arc<AppState>,
) -> AppResult<Uuid> {
    use sha2::{Digest, Sha256};

    log::info!("Starting server authentication handshake");

    let hello_msg = recv_control_message(&mut recv).await?;
    let hello: AuthHello = hello_msg.payload()?;

    log::info!(
        "Received auth hello from device {} ({})",
        hello.device_id,
        hello.device_name
    );

    if hello.protocol_version != PROTOCOL_VERSION {
        return Err(crate::core::AppError::Other(format!(
            "Incompatible protocol version: {}",
            hello.protocol_version
        )));
    }
    if hello.cert_fingerprint.is_empty() || hello.cert_fingerprint.starts_with("mock-") {
        return Err(crate::core::AppError::Other(
            "Peer certificate fingerprint is invalid".to_string(),
        ));
    }

    let response_nonce: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
    let signature =
        Sha256::digest(&[&hello.nonce[..], local_device_id.as_bytes()].concat()).to_vec();

    let cert_fingerprint = app_state.certificate_fingerprint().await.ok_or_else(|| {
        crate::core::AppError::Other("Local certificate fingerprint is unavailable".to_string())
    })?;

    let response = AuthResponse {
        device_id: local_device_id,
        device_name: local_device_name.clone(),
        cert_fingerprint,
        signature,
        nonce: response_nonce.clone(),
    };

    let response_msg = ControlMessage::new(ControlMessageType::AuthResponse, &response)?;
    send_control_message(&mut send, &response_msg).await?;

    let ack_msg = recv_control_message(&mut recv).await?;
    let ack: AuthAck = ack_msg.payload()?;

    let expected_ack =
        Sha256::digest(&[&response_nonce[..], hello.device_id.as_bytes()].concat()).to_vec();
    if !ack.authenticated || ack.device_id != hello.device_id || ack.signature != expected_ack {
        return Err(crate::core::AppError::Other(format!(
            "Authentication failed: {}",
            ack.error.unwrap_or_default()
        )));
    }

    upsert_device_info(
        &app_state,
        DeviceInfo {
            device_id: hello.device_id,
            device_name: hello.device_name.clone(),
            os: OperatingSystem::Unknown,
            ip_address: remote_addr.ip(),
            quic_port: remote_addr.port(),
            protocol_version: hello.protocol_version.clone(),
            capabilities: vec![],
            last_seen: std::time::SystemTime::now(),
            cert_fingerprint: hello.cert_fingerprint.clone(),
        },
    );

    log::info!(
        "Server authentication completed with device {}",
        hello.device_id
    );

    Ok(hello.device_id)
}

/// Merge authentication data into the device table without discarding
/// capabilities/OS information learned from discovery.
fn upsert_device_info(app_state: &Arc<AppState>, device: DeviceInfo) {
    use dashmap::mapref::entry::Entry;

    match app_state.devices.entry(device.device_id) {
        Entry::Occupied(mut entry) => {
            let existing = entry.get_mut();
            existing.ip_address = device.ip_address;
            existing.quic_port = device.quic_port;
            existing.last_seen = device.last_seen;
            if existing.cert_fingerprint.is_empty() {
                existing.cert_fingerprint = device.cert_fingerprint;
            }
            if existing.capabilities.is_empty() {
                existing.capabilities = device.capabilities;
            }
            if existing.device_name.is_empty() {
                existing.device_name = device.device_name;
            }
        }
        Entry::Vacant(entry) => {
            entry.insert(device);
        }
    }
}

/// Send control message with length prefix
async fn send_control_message(
    send: &mut quinn::SendStream,
    message: &ControlMessage,
) -> AppResult<()> {
    let data = message.to_bytes()?;
    let len = data.len() as u32;
    send.write_all(&len.to_be_bytes())
        .await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write message: {}", e)))?;
    send.write_all(&data)
        .await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to write message: {}", e)))?;
    Ok(())
}

/// Receive control message with length prefix
async fn recv_control_message(recv: &mut quinn::RecvStream) -> AppResult<ControlMessage> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await.map_err(|e| {
        crate::core::AppError::Network(format!("Failed to read message length: {}", e))
    })?;
    let msg_len = u32::from_be_bytes(len_buf) as usize;

    if msg_len > 10 * 1024 * 1024 {
        return Err(crate::core::AppError::Other(
            "Message too large".to_string(),
        ));
    }

    let mut msg_buf = vec![0u8; msg_len];
    recv.read_exact(&mut msg_buf)
        .await
        .map_err(|e| crate::core::AppError::Network(format!("Failed to read message: {}", e)))?;

    ControlMessage::from_bytes(&msg_buf)
}

/// Calculate certificate fingerprint
pub fn calculate_cert_fingerprint(cert_der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(cert_der);
    format!("{:x}", hash)
}

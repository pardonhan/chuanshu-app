use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::core::{AppError, AppResult, QUIC_IDLE_TIMEOUT, QUIC_KEEP_ALIVE_INTERVAL};

/// 局域网内使用自签名证书，客户端显式跳过证书校验。
///
/// 安全说明：任何能访问本机 QUIC 端口的主机都可以冒充设备。
/// 本应用定位为可信局域网内的个人工具，请勿在不可信网络中使用。
#[derive(Debug)]
struct NoCertificateVerification;

impl rustls::client::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::Certificate,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::Certificate,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

/// 生成自签名证书
pub fn generate_self_signed_cert() -> AppResult<(rustls::Certificate, rustls::PrivateKey)> {
    use rcgen::{Certificate, DistinguishedName, DnType};

    let mut params = rcgen::CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, "传书文件传输");
    params.subject_alt_names = vec![
        rcgen::SanType::DnsName("chuanshu.local".to_string()),
        rcgen::SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    ];

    let cert = Certificate::from_params(params)
        .map_err(|e| AppError::Other(format!("生成证书失败: {}", e)))?;

    let cert_der = cert
        .serialize_der()
        .map_err(|e| AppError::Other(format!("序列化证书失败: {}", e)))?;
    let key_der = cert.serialize_private_key_der();

    Ok((rustls::Certificate(cert_der), rustls::PrivateKey(key_der)))
}

/// QUIC 传输参数
fn transport_config() -> AppResult<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_bidi_streams(8u32.into());
    transport.max_concurrent_uni_streams(8u32.into());
    transport.receive_window((16u32 * 1024 * 1024).into());
    transport.stream_receive_window((8u32 * 1024 * 1024).into());
    transport.send_window(16 * 1024 * 1024);
    transport.keep_alive_interval(Some(Duration::from_secs(QUIC_KEEP_ALIVE_INTERVAL)));
    transport.max_idle_timeout(Some(
        Duration::from_secs(QUIC_IDLE_TIMEOUT)
            .try_into()
            .map_err(|_| AppError::Other("空闲超时参数非法".to_string()))?,
    ));
    Ok(transport)
}

/// 服务端 QUIC 配置（自签名证书）
pub fn server_config() -> AppResult<quinn::ServerConfig> {
    let (cert, key) = generate_self_signed_cert()?;

    let crypto = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| AppError::Other(format!("创建服务端 TLS 配置失败: {}", e)))?;

    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(Arc::new(transport_config()?));
    Ok(config)
}

/// 客户端 QUIC 配置（跳过证书校验）
pub fn client_config() -> quinn::ClientConfig {
    let crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
        .with_no_client_auth();

    let mut config = quinn::ClientConfig::new(Arc::new(crypto));
    // 传输参数构造失败时退回默认参数（不会阻断连接）
    if let Ok(transport) = transport_config() {
        config.transport_config(Arc::new(transport));
    }
    config
}

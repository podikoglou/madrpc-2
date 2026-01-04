use quinn::{ClientConfig, Connection, Endpoint, EndpointConfig};
use std::sync::Arc;
use crate::protocol::{Request, Response};
use crate::transport::codec::PostcardCodec;
use crate::protocol::error::Result;

/// QUIC transport wrapper for MaDRPC
pub struct QuicTransport {
    endpoint: Endpoint,
}

impl QuicTransport {
    /// Create a new client QUIC endpoint
    pub fn new_client() -> Result<Self> {
        // Install default crypto provider for rustls (ring backend)
        rustls::crypto::ring::default_provider().install_default().ok();

        // Create endpoint with default client config
        // For now, skip certificate verification (will be fixed later)
        let socket = std::net::UdpSocket::bind("[::]:0")?;
        let runtime = quinn::default_runtime()
            .ok_or_else(|| crate::protocol::error::MadrpcError::Connection("Failed to get runtime".to_string()))?;
        let mut endpoint = Endpoint::new(EndpointConfig::default(), None, socket, runtime)?;

        let crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_no_client_auth();

        let quic_config = quinn::crypto::rustls::QuicClientConfig::try_from(crypto)?;
        let config = ClientConfig::new(Arc::new(quic_config));
        endpoint.set_default_client_config(config);

        Ok(Self { endpoint })
    }

    /// Connect to a remote endpoint
    pub async fn connect(&self, addr: &str) -> Result<Connection> {
        let connecting = self.endpoint.connect(addr.parse()?, "madrpc")?;
        let connection = connecting.await?;
        Ok(connection)
    }

    /// Send a request and wait for response
    pub async fn send_request(
        &self,
        connection: &Connection,
        request: &Request,
    ) -> Result<Response> {
        // Open bidirectional stream
        let (mut send, mut recv) = connection.open_bi().await?;

        // Encode and send request
        let encoded = PostcardCodec::encode_request(request)?;

        // Send length prefix + data
        let len = encoded.len() as u32;
        send.write_all(&len.to_be_bytes()).await?;
        send.write_all(&encoded).await?;
        send.finish()?;

        // Read response length
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        // Read response data
        let mut buf = vec![0u8; len];
        recv.read_exact(&mut buf).await?;

        // Decode response
        let response = PostcardCodec::decode_response(&buf)?;

        Ok(response)
    }
}

// Temporary: Skip server verification for development
#[derive(Debug)]
struct SkipServerVerification;

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA1,
            rustls::SignatureScheme::ECDSA_SHA1_Legacy,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_quic_transport_creation() {
        // Install default crypto provider for rustls
        let _ = rustls::crypto::ring::default_provider().install_default();

        let transport = QuicTransport::new_client();
        match &transport {
            Ok(_) => {},
            Err(e) => eprintln!("Failed to create QUIC transport: {}", e),
        }
        assert!(transport.is_ok());
    }

    // For now, actual send/receive tests are deferred to integration tests
    // because they require a running QUIC server
}

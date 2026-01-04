use quinn::{Endpoint, ServerConfig};
use std::sync::Arc;
use crate::protocol::{Request, Response};
use crate::transport::codec::PostcardCodec;
use crate::protocol::error::Result;
use futures_util::future::FutureExt;

/// QUIC server for MaDRPC
pub struct QuicServer {
    endpoint: Endpoint,
}

impl QuicServer {
    /// Create a new QUIC server
    pub fn new(bind_addr: &str) -> Result<Self> {
        // Install crypto provider if not already installed
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Generate a self-signed certificate
        let cert = rcgen::generate_simple_self_signed(vec!["madrpc".to_string()]).unwrap();
        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.key_pair.serialize_der();

        let cert = rustls::pki_types::CertificateDer::from(cert_der);
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(key_der.into());

        // Build server config
        let crypto = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .map_err(|e| crate::protocol::error::MadrpcError::Connection(format!("Failed to build server config: {}", e)))?;

        let quic_config = quinn::crypto::rustls::QuicServerConfig::try_from(crypto)
            .map_err(|e| crate::protocol::error::MadrpcError::Connection(format!("Failed to build QUIC config: {}", e)))?;

        let server_config = ServerConfig::with_crypto(Arc::new(quic_config));

        // Bind to address
        let socket = std::net::UdpSocket::bind(bind_addr)
            .map_err(|e| crate::protocol::error::MadrpcError::Connection(format!("Failed to bind to {}: {}", bind_addr, e)))?;

        let runtime = quinn::default_runtime()
            .ok_or_else(|| crate::protocol::error::MadrpcError::Connection("Failed to get runtime".to_string()))?;

        let endpoint = Endpoint::new(quinn::EndpointConfig::default(), Some(server_config), socket, runtime)
            .map_err(|e| crate::protocol::error::MadrpcError::Connection(format!("Failed to create endpoint: {}", e)))?;

        Ok(Self { endpoint })
    }

    /// Get the actual bound address
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.endpoint.local_addr()
            .map_err(|e| crate::protocol::error::MadrpcError::Connection(format!("Failed to get local addr: {}", e)))
    }

    /// Accept incoming connections and handle requests
    /// This runs indefinitely until an error occurs
    pub async fn run_with_handler<F, Fut>(&self, handler: F) -> Result<()>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: std::future::IntoFuture<Output = Result<Response>> + Send + 'static,
        <Fut as std::future::IntoFuture>::IntoFuture: Send,
    {
        let handler = Arc::new(handler);

        while let Some(conn) = self.endpoint.accept().await {
            let handler = handler.clone();
            tokio::spawn(async move {
                match conn.await {
                    Ok(connection) => {
                        eprintln!("Connection established from {:?}", connection.remote_address());

                        // Handle bidirectional streams
                        while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                            let handler = handler.clone();
                            tokio::spawn(async move {
                                // Read request length
                                let mut len_buf = [0u8; 4];
                                if recv.read_exact(&mut len_buf).await.is_err() {
                                    eprintln!("Failed to read request length");
                                    return;
                                }

                                let len = u32::from_be_bytes(len_buf) as usize;

                                // Read request data
                                let mut buf = vec![0u8; len];
                                if recv.read_exact(&mut buf).await.is_err() {
                                    eprintln!("Failed to read request data");
                                    return;
                                }

                                // Decode request
                                let request = match PostcardCodec::decode_request(&buf) {
                                    Ok(req) => req,
                                    Err(e) => {
                                        eprintln!("Failed to decode request: {}", e);
                                        let error_response = Response::error(0, e.to_string());
                                        let _ = send_response(&mut send, &error_response).await;
                                        return;
                                    }
                                };

                                // Handle request asynchronously
                                let request_id = request.id;
                                let response_fut = handler(request);
                                let response = match response_fut.await {
                                    Ok(resp) => resp,
                                    Err(e) => {
                                        eprintln!("Handler error: {}", e);
                                        Response::error(request_id, e.to_string())
                                    }
                                };

                                // Send response
                                if let Err(e) = send_response(&mut send, &response).await {
                                    eprintln!("Failed to send response: {}", e);
                                }
                            });
                        }
                    }
                    Err(e) => {
                        eprintln!("Connection failed: {}", e);
                    }
                }
            });
        }

        Ok(())
    }
}

async fn send_response(send: &mut quinn::SendStream, response: &Response) -> Result<()> {
    let encoded = PostcardCodec::encode_response(response)?;

    // Send length prefix + data
    let len = encoded.len() as u32;
    send.write_all(&len.to_be_bytes()).await
        .map_err(|e| crate::protocol::error::MadrpcError::Connection(format!("Failed to send response length: {}", e)))?;
    send.write_all(&encoded).await
        .map_err(|e| crate::protocol::error::MadrpcError::Connection(format!("Failed to send response data: {}", e)))?;
    send.finish()
        .map_err(|e| crate::protocol::error::MadrpcError::Connection(format!("Failed to finish stream: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_creation() {
        // Install crypto provider for tests
        let _ = rustls::crypto::ring::default_provider().install_default();

        let server = QuicServer::new("0.0.0.0:0");
        if let Err(e) = &server {
            eprintln!("Server creation error: {:?}", e);
        }
        assert!(server.is_ok());
    }
}

// SPDX-License-Identifier: MPL-2.0
//! WebSocket abstraction, so the connection loops run on scripted frames in
//! tests. The real transport (feature `ws`) is tokio-tungstenite over
//! tokio-rustls with the ring provider and the Mozilla roots of
//! `webpki-roots`: no OpenSSL, no aws-lc.

use pa_twitch::BoxFuture;

use crate::source::ChatError;

/// Opens WebSocket connections.
pub trait WsConnector: Send + Sync {
    /// Connects to a `wss://` URL.
    fn connect<'a>(
        &'a self,
        url: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn WsConnection>, ChatError>>;
}

/// One open WebSocket. Only text frames matter for Twitch chat.
pub trait WsConnection: Send {
    /// Sends a text frame.
    fn send_text(&mut self, text: String) -> BoxFuture<'_, Result<(), ChatError>>;
    /// Next text frame; `None` once the peer closed the socket. Control
    /// frames (ping/pong) are handled inside.
    fn recv_text(&mut self) -> BoxFuture<'_, Option<Result<String, ChatError>>>;
    /// Closes the socket politely (best effort).
    fn close(&mut self) -> BoxFuture<'_, ()>;
}

#[cfg(feature = "ws")]
pub use live::TungsteniteConnector;

#[cfg(feature = "ws")]
mod live {
    use std::sync::Arc;

    use futures_util::{SinkExt, StreamExt};
    use pa_twitch::BoxFuture;
    use rustls::pki_types::ServerName;
    use tokio::net::TcpStream;
    use tokio_rustls::TlsConnector;
    use tokio_rustls::client::TlsStream;
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

    use super::{WsConnection, WsConnector};
    use crate::source::ChatError;

    /// Largest frame accepted; Twitch chat frames are a few KiB.
    const MAX_MESSAGE: usize = 1 << 20;
    /// TCP, TLS and the WebSocket handshake together (BUG-035: without a
    /// limit a stalled handshake never ended).
    const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
    /// One TCP connect to one resolved address.
    const ADDRESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    /// `wss://` connector: TCP, rustls (ring, TLS 1.2/1.3, webpki roots),
    /// then the WebSocket handshake.
    #[derive(Clone)]
    pub struct TungsteniteConnector {
        tls: Arc<rustls::ClientConfig>,
    }

    impl std::fmt::Debug for TungsteniteConnector {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TungsteniteConnector")
                .finish_non_exhaustive()
        }
    }

    impl TungsteniteConnector {
        /// Connector trusting the Mozilla root store.
        pub fn new() -> Result<Self, ChatError> {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let tls = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| ChatError::Connect(e.to_string()))?
                .with_root_certificates(roots)
                .with_no_client_auth();
            Ok(Self { tls: Arc::new(tls) })
        }
    }

    impl WsConnector for TungsteniteConnector {
        fn connect<'a>(
            &'a self,
            url: &'a str,
        ) -> BoxFuture<'a, Result<Box<dyn WsConnection>, ChatError>> {
            Box::pin(async move {
                tokio::time::timeout(CONNECT_TIMEOUT, self.open(url))
                    .await
                    .unwrap_or_else(|_| Err(ChatError::Connect("connection timed out".to_owned())))
            })
        }
    }

    impl TungsteniteConnector {
        async fn open(&self, url: &str) -> Result<Box<dyn WsConnection>, ChatError> {
            let request = url
                .into_client_request()
                .map_err(|e| ChatError::Connect(e.to_string()))?;
            let uri = request.uri();
            if uri.scheme_str() != Some("wss") {
                return Err(ChatError::Connect("only wss:// is allowed".to_owned()));
            }
            let host = uri
                .host()
                .ok_or_else(|| ChatError::Connect("URL without host".to_owned()))?
                .to_owned();
            let port = uri.port_u16().unwrap_or(443);
            let tcp = tcp_connect(&host, port).await?;
            let _ = tcp.set_nodelay(true);
            let name = ServerName::try_from(host).map_err(|e| ChatError::Connect(e.to_string()))?;
            let tls = TlsConnector::from(self.tls.clone())
                .connect(name, tcp)
                .await
                .map_err(|e| ChatError::Connect(e.to_string()))?;
            let mut config = WebSocketConfig::default();
            config.max_message_size = Some(MAX_MESSAGE);
            config.max_frame_size = Some(MAX_MESSAGE);
            let (ws, _) = tokio_tungstenite::client_async_with_config(request, tls, Some(config))
                .await
                .map_err(|e| ChatError::Connect(e.to_string()))?;
            Ok(Box::new(Conn { ws }) as Box<dyn WsConnection>)
        }
    }

    /// Tries each address of `host` with its own short timeout, so one
    /// unreachable address (e.g. a broken IPv6 route) does not use up the
    /// whole attempt.
    async fn tcp_connect(host: &str, port: u16) -> Result<TcpStream, ChatError> {
        let addrs = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| ChatError::Connect(e.to_string()))?;
        let mut last = "no address".to_owned();
        for addr in addrs {
            match tokio::time::timeout(ADDRESS_TIMEOUT, TcpStream::connect(addr)).await {
                Ok(Ok(tcp)) => return Ok(tcp),
                Ok(Err(e)) => last = e.to_string(),
                Err(_) => last = format!("{addr}: timed out"),
            }
        }
        Err(ChatError::Connect(last))
    }

    struct Conn {
        ws: WebSocketStream<TlsStream<TcpStream>>,
    }

    impl WsConnection for Conn {
        fn send_text(&mut self, text: String) -> BoxFuture<'_, Result<(), ChatError>> {
            Box::pin(async move {
                self.ws
                    .send(Message::Text(text.into()))
                    .await
                    .map_err(|e| ChatError::Socket(e.to_string()))
            })
        }

        fn recv_text(&mut self) -> BoxFuture<'_, Option<Result<String, ChatError>>> {
            Box::pin(async move {
                loop {
                    match self.ws.next().await? {
                        Ok(Message::Text(t)) => return Some(Ok(t.as_str().to_owned())),
                        Ok(Message::Binary(b)) => {
                            return Some(Ok(String::from_utf8_lossy(&b).into_owned()));
                        }
                        Ok(Message::Close(_)) => return None,
                        // Pings are answered by tungstenite on the next poll.
                        Ok(_) => continue,
                        Err(e) => return Some(Err(ChatError::Socket(e.to_string()))),
                    }
                }
            })
        }

        fn close(&mut self) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                let _ = self.ws.close(None).await;
            })
        }
    }
}

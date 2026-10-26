//! WebSocket client connection.

use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_tls_with_config};

pub use tokio_tungstenite::tungstenite::Error as WsError;

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Connects to a `ws://` or `wss://` URL with Nagle disabled. TLS uses the process-level rustls
/// crypto provider, which `main` installs first thing.
pub async fn connect(url: &str) -> Result<WsStream, WsError> {
    let (ws, _response) = connect_async_tls_with_config(url, None, true, None).await?;
    Ok(ws)
}

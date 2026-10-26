//! Live push: the current [`LiveSnapshot`] on connect, then one frame per change.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use tokio::sync::watch;

use crate::AppState;

pub async fn ws_handler(State(s): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| push_live(socket, s.live))
}

async fn push_live(mut socket: WebSocket, mut live: watch::Receiver<raki_core::LiveSnapshot>) {
    loop {
        // Serialize before awaiting: the watch guard must not be held across an await.
        let json = match serde_json::to_string(&*live.borrow_and_update()) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "serialize live snapshot");
                return;
            }
        };
        if socket.send(Message::Text(json.into())).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                changed = live.changed() => {
                    if changed.is_err() {
                        return; // sender gone: the app is shutting down
                    }
                    break; // send the new snapshot
                }
                msg = socket.recv() => match msg {
                    // Client messages are ignored; pings are answered by axum itself.
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                    Some(Ok(_)) => {}
                },
            }
        }
    }
}

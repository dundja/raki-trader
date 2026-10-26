//! HTTP and WebSocket API for the dashboard.
//!
//! The API owns no state of its own. Reads go to the [`Store`]; the live picture is a
//! `watch` channel the traderd shell keeps current; control commands are forwarded over an
//! `mpsc` channel to the shell, which owns the control state machine and answers on a oneshot.

pub mod routes;
pub mod ws;

use axum::Router;
use axum::http::{HeaderValue, Method, header};
use axum::routing::{get, post};
use raki_core::{ControlResponse, LiveSnapshot};
use raki_store::Store;
use tokio::sync::{mpsc, oneshot, watch};
use tower_http::cors::CorsLayer;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlCommand {
    Pause { reason: Option<String> },
    Resume { acknowledge: bool },
    Kill { reason: Option<String> },
}

/// A control command plus the channel the shell answers on.
pub struct ControlMsg {
    pub cmd: ControlCommand,
    pub reply: oneshot::Sender<ControlResponse>,
}

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub live: watch::Receiver<LiveSnapshot>,
    pub control: mpsc::Sender<ControlMsg>,
}

/// Builds the API router. `cors_origin` is the single allowed dashboard origin.
///
/// # Panics
/// If `cors_origin` is not a valid header value; that is a config error and must fail at start.
pub fn router(state: AppState, cors_origin: &str) -> Router {
    let origin: HeaderValue = cors_origin
        .parse()
        .expect("cors_origin must be a valid header value");
    let cors = CorsLayer::new()
        .allow_origin(origin)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE]);
    Router::new()
        .route("/api/live", get(routes::live))
        .route("/api/health", get(routes::health))
        .route("/api/stats", get(routes::stats))
        .route("/api/windows", get(routes::list_windows))
        .route("/api/windows/{slug}", get(routes::get_window))
        .route("/api/windows/{slug}/series", get(routes::get_series))
        .route("/api/control/pause", post(routes::pause))
        .route("/api/control/resume", post(routes::resume))
        .route("/api/control/kill", post(routes::kill))
        .route("/api/ws", get(ws::ws_handler))
        .layer(cors)
        .with_state(state)
}

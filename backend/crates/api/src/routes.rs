//! JSON route handlers. Errors are `{"error": "..."}` with the matching status.

use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use raki_core::{
    Asset, ControlRequest, ControlResponse, HealthSnapshot, LiveSnapshot, RecordingStats,
    SeriesRow, WindowRecord,
};
use serde::Deserialize;
use tokio::sync::oneshot;

use crate::{AppState, ControlCommand, ControlMsg};

/// How long a control request waits for the shell to answer.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 500;

pub enum ApiError {
    BadRequest(String),
    NotFound(String),
    Unavailable(String),
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, m),
            ApiError::Internal(e) => {
                // The detail may name SQL or paths; log it, never send it.
                tracing::error!(error = ?e, "api internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError::Internal(e)
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

pub async fn live(State(s): State<AppState>) -> Json<LiveSnapshot> {
    Json(s.live.borrow().clone())
}

pub async fn health(State(s): State<AppState>) -> Json<HealthSnapshot> {
    Json(s.live.borrow().health.clone())
}

pub async fn stats(State(s): State<AppState>) -> ApiResult<RecordingStats> {
    Ok(Json(s.store.recording_stats().await?))
}

#[derive(Deserialize)]
pub struct WindowsQuery {
    asset: Option<String>,
    limit: Option<i64>,
    before: Option<i64>,
}

pub async fn list_windows(
    State(s): State<AppState>,
    q: Result<Query<WindowsQuery>, QueryRejection>,
) -> ApiResult<Vec<WindowRecord>> {
    let Query(q) = q.map_err(|e| ApiError::BadRequest(e.body_text()))?;
    let asset = match q.asset.as_deref() {
        None => None,
        Some(a) => Some(
            Asset::from_any(a)
                .ok_or_else(|| ApiError::BadRequest(format!("unknown asset: {a}")))?,
        ),
    };
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    Ok(Json(s.store.list_windows(asset, limit, q.before).await?))
}

pub async fn get_window(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<WindowRecord> {
    match s.store.get_window(&slug).await? {
        Some(w) => Ok(Json(w)),
        None => Err(ApiError::NotFound(format!("window not found: {slug}"))),
    }
}

pub async fn get_series(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Vec<SeriesRow>> {
    Ok(Json(s.store.get_series(&slug).await?))
}

pub async fn pause(State(s): State<AppState>, body: Bytes) -> ApiResult<ControlResponse> {
    let req = parse_request(&body)?;
    send_control(&s, ControlCommand::Pause { reason: req.reason }).await
}

pub async fn resume(State(s): State<AppState>, body: Bytes) -> ApiResult<ControlResponse> {
    let req = parse_request(&body)?;
    send_control(
        &s,
        ControlCommand::Resume {
            acknowledge: req.acknowledge.unwrap_or(false),
        },
    )
    .await
}

pub async fn kill(State(s): State<AppState>, body: Bytes) -> ApiResult<ControlResponse> {
    let req = parse_request(&body)?;
    send_control(&s, ControlCommand::Kill { reason: req.reason }).await
}

/// The body is optional: an empty body means no reason and no acknowledgement.
fn parse_request(body: &[u8]) -> Result<ControlRequest, ApiError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(ControlRequest {
            reason: None,
            acknowledge: None,
        });
    }
    serde_json::from_slice(body).map_err(|e| ApiError::BadRequest(format!("invalid body: {e}")))
}

/// Forwards the command and waits for the shell's answer. A refusal (`ok == false`) is still a
/// 200; only "nobody answered in time" is a 503.
async fn send_control(s: &AppState, cmd: ControlCommand) -> ApiResult<ControlResponse> {
    let (reply, rx) = oneshot::channel();
    let exchange = async {
        s.control.send(ControlMsg { cmd, reply }).await.ok()?;
        rx.await.ok()
    };
    match tokio::time::timeout(CONTROL_TIMEOUT, exchange).await {
        Ok(Some(resp)) => Ok(Json(resp)),
        _ => Err(ApiError::Unavailable(
            "control handler did not respond".into(),
        )),
    }
}

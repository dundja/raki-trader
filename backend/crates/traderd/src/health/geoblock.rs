//! Periodic Polymarket geoblock check.

use std::sync::Arc;
use std::time::Duration;

use raki_feeds::rest::PolyRest;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::{HealthState, woke};

/// Checks now, then every `interval` and right after every wake from sleep (`wake`: the network,
/// and so the country, may have changed), until shutdown. A failed check makes the status
/// unknown, which the safety rules treat as not ok: trading must not rely on a stale "not
/// blocked".
pub async fn run_geoblock_monitor(
    state: Arc<HealthState>,
    rest: PolyRest,
    interval: Duration,
    mut wake: broadcast::Receiver<()>,
    shutdown: CancellationToken,
) {
    loop {
        let result = tokio::select! {
            _ = shutdown.cancelled() => return,
            r = rest.geoblock() => r,
        };
        match result {
            Ok(g) => {
                if g.blocked {
                    tracing::error!(country = %g.country, "geoblocked");
                }
                state.set_geo(Some(g.blocked), Some(g.country));
            }
            Err(e) => {
                tracing::warn!(error = %e, "geoblock check failed; status unknown");
                state.set_geo(None, None);
            }
        }
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(interval) => {}
            () = woke(&mut wake) => tracing::info!("woke from sleep; rechecking the geoblock"),
        }
    }
}

#[cfg(test)]
mod tests {
    use raki_feeds::rest::RestUrls;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn rest(server: &MockServer) -> PolyRest {
        let base = server.uri();
        PolyRest::new(
            RestUrls {
                gamma_base: base.clone(),
                data_base: base.clone(),
                geoblock_url: format!("{base}/api/geoblock"),
            },
            None,
        )
    }

    async fn wait_for_check(state: &HealthState) {
        for _ in 0..200 {
            if state.geo().checked_ms.is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("geoblock monitor never ran a check");
    }

    #[tokio::test]
    async fn monitor_stores_geoblock_result() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/geoblock"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"blocked":true,"ip":"203.0.113.7","country":"US","region":"NY"}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let state = HealthState::new(50.0);
        let shutdown = CancellationToken::new();
        let h = tokio::spawn(run_geoblock_monitor(
            state.clone(),
            rest(&server),
            Duration::from_secs(3600),
            broadcast::channel(1).1,
            shutdown.clone(),
        ));
        wait_for_check(&state).await;
        let g = state.geo();
        assert_eq!((g.blocked, g.country.as_deref()), (Some(true), Some("US")));
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), h)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn failed_check_leaves_status_unknown() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let state = HealthState::new(50.0);
        state.set_geo(Some(false), Some("RS".into()));
        let shutdown = CancellationToken::new();
        let h = tokio::spawn(run_geoblock_monitor(
            state.clone(),
            rest(&server),
            Duration::from_secs(3600),
            broadcast::channel(1).1,
            shutdown.clone(),
        ));
        for _ in 0..200 {
            if state.geo().blocked.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(state.geo().blocked, None);
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), h)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn wake_triggers_a_recheck() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/geoblock"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"blocked":false,"ip":"203.0.113.7","country":"RS","region":"00"}"#,
                "application/json",
            ))
            .mount(&server)
            .await;
        let state = HealthState::new(50.0);
        let shutdown = CancellationToken::new();
        let (wake_tx, wake_rx) = broadcast::channel(1);
        let h = tokio::spawn(run_geoblock_monitor(
            state.clone(),
            rest(&server),
            Duration::from_secs(3600),
            wake_rx,
            shutdown.clone(),
        ));
        wait_for_check(&state).await;
        wake_tx.send(()).unwrap();
        let mut n = 0;
        for _ in 0..200 {
            n = server.received_requests().await.unwrap().len();
            if n >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(n, 2, "one check at start, one on wake");
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), h)
            .await
            .unwrap()
            .unwrap();
    }
}

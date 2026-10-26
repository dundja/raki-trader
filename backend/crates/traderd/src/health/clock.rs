//! Local clock offset against NTP servers.

use std::sync::Arc;
use std::time::Duration;

use rsntp::{AsyncSntpClient, Config};
use tokio_util::sync::CancellationToken;

use super::{HealthState, median_offset_ms};

const SERVER_TIMEOUT: Duration = Duration::from_secs(2);

/// Queries each server and returns the offsets that answered, in milliseconds. Positive means the
/// local clock is behind. Servers are asked one after another, each with its own 2 s timeout.
async fn query_offsets(servers: &[String]) -> Vec<f64> {
    let client = AsyncSntpClient::with_config(Config::default().timeout(SERVER_TIMEOUT));
    let mut offsets = Vec::with_capacity(servers.len());
    for server in servers {
        // rsntp's timeout only covers the wait for the reply; the outer one also covers DNS.
        match tokio::time::timeout(SERVER_TIMEOUT, client.synchronize(server.as_str())).await {
            Ok(Ok(r)) => offsets.push(r.clock_offset().as_secs_f64() * 1000.0),
            Ok(Err(e)) => tracing::warn!(%server, error = %e, "ntp query failed"),
            Err(_) => tracing::warn!(%server, "ntp query timed out"),
        }
    }
    offsets
}

/// Measures the median offset now, then every `interval`, until shutdown.
pub async fn run_clock_monitor(
    state: Arc<HealthState>,
    servers: Vec<String>,
    interval: Duration,
    shutdown: CancellationToken,
) {
    loop {
        let offsets = tokio::select! {
            _ = shutdown.cancelled() => return,
            o = query_offsets(&servers) => o,
        };
        let median = median_offset_ms(&offsets);
        if median.is_none() {
            tracing::warn!("no ntp server answered; clock offset unknown");
        }
        state.set_clock_offset(median);
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(interval) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "needs network access to public NTP servers"]
    async fn live_clock_offset() {
        let servers = vec!["time.cloudflare.com".to_owned(), "pool.ntp.org".to_owned()];
        let offset = median_offset_ms(&query_offsets(&servers).await);
        let offset = offset.expect("at least one ntp server answers");
        assert!(offset.abs() < 5000.0, "offset {offset} ms");
    }

    #[tokio::test]
    async fn monitor_without_answers_marks_clock_unknown_and_stops() {
        let state = HealthState::new(50.0);
        let shutdown = CancellationToken::new();
        let h = tokio::spawn(run_clock_monitor(
            state.clone(),
            vec![],
            Duration::from_secs(3600),
            shutdown.clone(),
        ));
        for _ in 0..100 {
            if state.clock().checked_ms.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let c = state.clock();
        assert!(c.checked_ms.is_some() && c.offset_ms.is_none() && !c.ok);
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), h)
            .await
            .unwrap()
            .unwrap();
    }
}

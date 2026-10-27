//! Stop signals and the bounded wait for tasks to finish.

use std::time::Duration;

use tokio::task::JoinSet;

/// Waits for Ctrl-C (SIGINT) or SIGTERM and names the one that arrived.
pub async fn signal() -> &'static str {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        r = tokio::signal::ctrl_c() => {
            r.expect("install Ctrl-C handler");
            "SIGINT"
        }
        _ = term.recv() => "SIGTERM",
    }
}

/// Waits up to `within` for every task to end, then aborts the rest. Aborting drops whatever a
/// task holds, recorder handles included, so the recorder can finish its file. Returns how many
/// tasks had to be aborted.
pub async fn join_within(mut tasks: JoinSet<()>, within: Duration) -> usize {
    let joined = tokio::time::timeout(within, async {
        while let Some(r) = tasks.join_next().await {
            if let Err(e) = r
                && e.is_panic()
            {
                tracing::error!(error = %e, "task panicked");
            }
        }
    })
    .await;
    if joined.is_ok() {
        return 0;
    }
    let left = tasks.len();
    tracing::warn!(tasks = left, "tasks did not stop in time; aborting");
    tasks.shutdown().await;
    left
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use tokio::task::JoinSet;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[tokio::test]
    async fn join_within_aborts_tasks_that_overrun() {
        let token = CancellationToken::new();
        let mut tasks = JoinSet::new();
        let t = token.clone();
        tasks.spawn(async move { t.cancelled().await });
        tasks.spawn(std::future::pending::<()>());
        token.cancel();
        let started = Instant::now();
        let aborted = join_within(tasks, Duration::from_millis(200)).await;
        assert_eq!(aborted, 1);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn join_within_returns_early_when_all_finish() {
        let mut tasks = JoinSet::new();
        tasks.spawn(async {});
        let started = Instant::now();
        assert_eq!(join_within(tasks, Duration::from_secs(5)).await, 0);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

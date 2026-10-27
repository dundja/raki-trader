//! Stop signals, task supervision and the bounded wait for tasks to finish.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::time::Duration;

use tokio::task::{Id, JoinHandle, JoinSet};

/// The long-running tasks, by name. Every one of them runs until shutdown: one that ends earlier
/// (returned or panicked) means part of the recorder silently stopped working.
#[derive(Default)]
pub struct Tasks {
    set: JoinSet<()>,
    names: HashMap<Id, &'static str>,
}

impl Tasks {
    pub fn spawn(&mut self, name: &'static str, task: impl Future<Output = ()> + Send + 'static) {
        let id = self.set.spawn(task).id();
        self.names.insert(id, name);
    }

    pub fn into_join_set(self) -> JoinSet<()> {
        self.set
    }
}

/// Why the process is stopping.
#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    /// SIGINT or SIGTERM.
    Signal(&'static str),
    /// A task (or the raw recorder) ended before shutdown; the text names it and says how.
    Failed(String),
}

impl Stop {
    /// The session's end reason: `shutdown`, or `task_failed:<what>`.
    pub fn reason(&self) -> String {
        match self {
            Stop::Signal(_) => "shutdown".to_owned(),
            Stop::Failed(what) => format!("task_failed:{what}"),
        }
    }
}

/// Waits for a stop signal, or for any task or the raw recorder to end on its own, whichever
/// comes first. If the recorder ended, its result is moved out of `recorder` (a finished handle
/// must not be awaited again).
pub async fn wait_for_stop(
    tasks: &mut Tasks,
    recorder: &mut Option<JoinHandle<io::Result<()>>>,
    signal: impl Future<Output = &'static str>,
) -> Stop {
    let recorder_ended = async {
        match recorder.as_mut() {
            Some(h) => h.await,
            None => std::future::pending().await,
        }
    };
    let stop = tokio::select! {
        biased;
        sig = signal => Stop::Signal(sig),
        Some(joined) = tasks.set.join_next_with_id() => {
            let (id, how) = match joined {
                Ok((id, ())) => (id, "returned".to_owned()),
                Err(e) => (e.id(), if e.is_panic() { "panicked".to_owned() } else { e.to_string() }),
            };
            let name = tasks.names.get(&id).copied().unwrap_or("unknown");
            Stop::Failed(format!("{name} {how}"))
        }
        r = recorder_ended => {
            let how = match r {
                Ok(Ok(())) => "recorder stopped".to_owned(),
                Ok(Err(e)) => format!("recorder I/O error: {e}"),
                Err(e) => format!("recorder task failed: {e}"),
            };
            Stop::Failed(how)
        }
    };
    if matches!(&stop, Stop::Failed(w) if w.starts_with("recorder")) {
        *recorder = None;
    }
    stop
}

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

    fn never() -> impl Future<Output = &'static str> {
        std::future::pending()
    }

    #[tokio::test]
    async fn a_task_ending_early_is_a_failure_naming_it() {
        let mut tasks = Tasks::default();
        tasks.spawn("clob-btc", std::future::pending());
        tasks.spawn("engine", async {});
        let mut recorder = None;
        let stop = wait_for_stop(&mut tasks, &mut recorder, never()).await;
        assert_eq!(stop, Stop::Failed("engine returned".into()));
        assert_eq!(stop.reason(), "task_failed:engine returned");
    }

    #[tokio::test]
    async fn a_panicking_task_is_a_failure() {
        let mut tasks = Tasks::default();
        tasks.spawn("clock", async { panic!("boom") });
        let stop = wait_for_stop(&mut tasks, &mut None, never()).await;
        assert_eq!(stop, Stop::Failed("clock panicked".into()));
    }

    #[tokio::test]
    async fn a_recorder_io_error_is_a_failure_and_its_handle_is_taken() {
        let mut tasks = Tasks::default();
        tasks.spawn("engine", std::future::pending());
        let mut recorder = Some(tokio::spawn(async {
            Err(io::Error::new(io::ErrorKind::StorageFull, "disk full"))
        }));
        let stop = wait_for_stop(&mut tasks, &mut recorder, never()).await;
        assert!(
            matches!(&stop, Stop::Failed(w) if w.contains("disk full")),
            "{stop:?}"
        );
        assert!(recorder.is_none());
    }

    #[tokio::test]
    async fn a_signal_is_a_clean_stop() {
        let mut tasks = Tasks::default();
        tasks.spawn("engine", std::future::pending());
        let mut recorder = Some(tokio::spawn(std::future::pending::<io::Result<()>>()));
        let stop = wait_for_stop(&mut tasks, &mut recorder, async { "SIGTERM" }).await;
        assert_eq!(stop, Stop::Signal("SIGTERM"));
        assert_eq!(stop.reason(), "shutdown");
        assert!(recorder.is_some());
    }

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

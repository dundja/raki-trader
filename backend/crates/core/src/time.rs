use std::time::{SystemTime, UNIX_EPOCH};

fn since_epoch() -> std::time::Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the unix epoch")
}

/// Local wall-clock time as epoch nanoseconds (UTC).
pub fn now_ns() -> i64 {
    since_epoch().as_nanos() as i64
}

/// Local wall-clock time as epoch milliseconds (UTC).
pub fn now_ms() -> i64 {
    since_epoch().as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ns_and_ms_agree() {
        let ms = now_ms();
        let ns = now_ns();
        assert!(ms > 1_700_000_000_000);
        assert!((ns / 1_000_000 - ms).abs() < 1_000);
    }
}

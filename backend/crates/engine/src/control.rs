//! The trading control state machine: Running / Paused / Killed.
//!
//! M1 places no orders; this only gates future trading and backs the dashboard controls.

use raki_core::{ControlState, SafetyChecks, StopKind, TradingMode};

#[derive(Debug)]
pub struct Control {
    state: ControlState,
}

impl Control {
    /// Starts Paused with [`StopKind::Startup`]; the first all-ok safety check starts trading.
    pub fn new(now_ms: i64) -> Control {
        Control {
            state: ControlState {
                mode: TradingMode::Paused,
                stop_kind: Some(StopKind::Startup),
                reason: None,
                needs_ack: false,
                since_ms: now_ms,
            },
        }
    }

    pub fn state(&self) -> ControlState {
        self.state.clone()
    }

    /// Stops opening positions (manual).
    pub fn pause(&mut self, reason: Option<String>, now_ms: i64) -> ControlState {
        self.stop(TradingMode::Paused, reason, now_ms)
    }

    /// Pause plus (from M3 on) cancel every resting order (manual).
    pub fn kill(&mut self, reason: Option<String>, now_ms: i64) -> ControlState {
        self.stop(TradingMode::Killed, reason, now_ms)
    }

    /// Back to Running from Paused or Killed. Refused while any safety check fails, and after
    /// a loss cap or config halt unless `acknowledge` is set.
    pub fn resume(
        &mut self,
        safety: &SafetyChecks,
        acknowledge: bool,
        now_ms: i64,
    ) -> Result<ControlState, String> {
        if !safety.all_ok {
            return Err(format!(
                "safety checks failing: {}",
                safety.reasons.join("; ")
            ));
        }
        if self.state.needs_ack && !acknowledge {
            let reason = self
                .state
                .reason
                .clone()
                .or_else(|| self.state.stop_kind.map(|k| format!("{k:?}")))
                .unwrap_or_default();
            return Err(format!("acknowledge required: {reason}"));
        }
        if self.state.mode != TradingMode::Running {
            self.state = running(now_ms);
        }
        Ok(self.state())
    }

    /// Called every tick: a failing check pauses a running bot; the first all-ok check after
    /// startup starts it (once — any later stop needs a manual resume).
    pub fn on_safety(&mut self, safety: &SafetyChecks, now_ms: i64) {
        match (self.state.mode, self.state.stop_kind) {
            (TradingMode::Running, _) if !safety.all_ok => {
                self.state = ControlState {
                    mode: TradingMode::Paused,
                    stop_kind: Some(StopKind::Safety),
                    reason: Some(safety.reasons.join("; ")),
                    needs_ack: false,
                    since_ms: now_ms,
                };
            }
            (TradingMode::Paused, Some(StopKind::Startup)) if safety.all_ok => {
                self.state = running(now_ms);
            }
            _ => {}
        }
    }

    /// A manual stop. A pending acknowledgement (loss cap, config halt) survives it, so pausing
    /// or killing can never be used to skip the acknowledgement.
    fn stop(&mut self, mode: TradingMode, reason: Option<String>, now_ms: i64) -> ControlState {
        if self.state.needs_ack {
            self.state.mode = mode;
            self.state.since_ms = now_ms;
        } else {
            self.state = ControlState {
                mode,
                stop_kind: Some(StopKind::Manual),
                reason,
                needs_ack: false,
                since_ms: now_ms,
            };
        }
        self.state()
    }

    #[cfg(test)]
    pub(crate) fn set_stopped(&mut self, kind: StopKind, reason: Option<String>, now_ms: i64) {
        self.state = ControlState {
            mode: TradingMode::Paused,
            stop_kind: Some(kind),
            reason,
            needs_ack: matches!(kind, StopKind::LossCap | StopKind::ConfigHalt),
            since_ms: now_ms,
        };
    }
}

fn running(now_ms: i64) -> ControlState {
    ControlState {
        mode: TradingMode::Running,
        stop_kind: None,
        reason: None,
        needs_ack: false,
        since_ms: now_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn safe() -> SafetyChecks {
        SafetyChecks {
            feeds_fresh: true,
            clock_ok: true,
            geoblock_ok: true,
            recorder_ok: true,
            store_ok: true,
            all_ok: true,
            reasons: vec![],
        }
    }

    fn unsafe_checks() -> SafetyChecks {
        SafetyChecks {
            feeds_fresh: false,
            clock_ok: false,
            all_ok: false,
            reasons: vec!["clob stale".into(), "clock offset 900 ms".into()],
            ..safe()
        }
    }

    #[test]
    fn new_is_paused_at_startup() {
        let s = Control::new(5).state();
        assert_eq!(s.mode, TradingMode::Paused);
        assert_eq!(s.stop_kind, Some(StopKind::Startup));
        assert!(!s.needs_ack);
        assert_eq!(s.since_ms, 5);
    }

    #[test]
    fn startup_auto_runs_once_when_safe() {
        let mut c = Control::new(0);
        c.on_safety(&unsafe_checks(), 1);
        assert_eq!(c.state().stop_kind, Some(StopKind::Startup));

        c.on_safety(&safe(), 2);
        let s = c.state();
        assert_eq!(
            (s.mode, s.stop_kind, s.since_ms),
            (TradingMode::Running, None, 2)
        );

        c.on_safety(&unsafe_checks(), 3);
        let s = c.state();
        assert_eq!(s.mode, TradingMode::Paused);
        assert_eq!(s.stop_kind, Some(StopKind::Safety));
        assert_eq!(s.reason.as_deref(), Some("clob stale; clock offset 900 ms"));
        assert!(!s.needs_ack);

        c.on_safety(&safe(), 4);
        assert_eq!(
            c.state().mode,
            TradingMode::Paused,
            "auto-start is one-time"
        );
        assert_eq!(c.state().since_ms, 3);
    }

    #[test]
    fn resume_refused_when_unsafe() {
        let mut c = Control::new(0);
        c.pause(Some("lunch".into()), 1);
        let err = c.resume(&unsafe_checks(), true, 2).unwrap_err();
        assert!(err.contains("safety checks failing"), "{err}");
        assert!(err.contains("clob stale; clock offset 900 ms"), "{err}");
        assert_eq!(c.state().mode, TradingMode::Paused);
    }

    #[test]
    fn resume_requires_ack_for_loss_cap() {
        let mut c = Control::new(0);
        c.set_stopped(StopKind::LossCap, Some("daily loss cap hit".into()), 1);
        assert!(c.state().needs_ack);

        let err = c.resume(&safe(), false, 2).unwrap_err();
        assert_eq!(err, "acknowledge required: daily loss cap hit");
        assert_eq!(c.state().mode, TradingMode::Paused);

        let s = c.resume(&safe(), true, 3).unwrap();
        assert_eq!(s.mode, TradingMode::Running);
        assert_eq!((s.stop_kind, s.reason, s.needs_ack), (None, None, false));
        assert_eq!(s.since_ms, 3);
    }

    #[test]
    fn config_halt_needs_ack_too() {
        let mut c = Control::new(0);
        c.set_stopped(StopKind::ConfigHalt, None, 1);
        let err = c.resume(&safe(), false, 2).unwrap_err();
        assert!(err.starts_with("acknowledge required: "), "{err}");
        assert!(c.resume(&safe(), true, 3).is_ok());
    }

    #[test]
    fn pause_does_not_clear_a_pending_ack() {
        let mut c = Control::new(0);
        c.set_stopped(StopKind::LossCap, Some("daily loss cap hit".into()), 1);
        c.pause(Some("manual".into()), 2);
        c.kill(None, 3);
        let s = c.state();
        assert_eq!(s.mode, TradingMode::Killed);
        assert_eq!(s.stop_kind, Some(StopKind::LossCap));
        assert!(s.needs_ack);
        assert!(c.resume(&safe(), false, 4).is_err());
    }

    #[test]
    fn kill_then_resume_ok_when_safe() {
        let mut c = Control::new(0);
        c.on_safety(&safe(), 1);
        let s = c.kill(Some("panic button".into()), 2);
        assert_eq!(s.mode, TradingMode::Killed);
        assert_eq!(s.stop_kind, Some(StopKind::Manual));
        assert_eq!(s.reason.as_deref(), Some("panic button"));
        assert_eq!(s.since_ms, 2);

        let s = c.resume(&safe(), false, 3).unwrap();
        assert_eq!(s.mode, TradingMode::Running);
        assert_eq!(c.state(), s);
    }

    #[test]
    fn manual_pause_is_not_auto_resumed() {
        let mut c = Control::new(0);
        let s = c.pause(None, 1);
        assert_eq!(
            (s.mode, s.stop_kind),
            (TradingMode::Paused, Some(StopKind::Manual))
        );
        c.on_safety(&safe(), 2);
        assert_eq!(c.state().mode, TradingMode::Paused);
    }
}

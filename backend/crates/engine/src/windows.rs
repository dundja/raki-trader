//! One tracked window: its record, lifecycle status and the REST polls it still needs.

use raki_core::{GammaWindow, OracleKind, Outcome, WindowId, WindowRecord, WindowStatus};

use crate::engine::Request;
use crate::oracle::OracleState;

/// Minimum gap between Gamma discovery requests for a window without a `condition_id`.
pub const DISCOVERY_INTERVAL_MS: i64 = 5_000;
/// Resolution polling starts this long after the window end...
pub const RESOLUTION_START_MS: i64 = 60_000;
/// ...and repeats at this interval until a winner is known.
pub const RESOLUTION_INTERVAL_MS: i64 = 5_000;
/// Gamma `finalPrice` polling starts this long after the window end...
pub const GAMMA_FINAL_START_MS: i64 = 120_000;
/// ...and repeats at this interval until `finalPrice` is known.
pub const GAMMA_FINAL_INTERVAL_MS: i64 = 30_000;
/// All polling stops this long after the window end; the window is then `TimedOut`.
pub const GIVE_UP_MS: i64 = 30 * 60_000;
/// A `Complete` or `TimedOut` window is forgotten this long after it got there.
pub const DROP_AFTER_MS: i64 = 60_000;

#[derive(Debug)]
pub struct WindowState {
    pub id: WindowId,
    pub rec: WindowRecord,
    /// The record as last emitted, to emit only on change.
    last_emitted: Option<WindowRecord>,
    last_gamma_req_ms: Option<i64>,
    last_resolution_req_ms: Option<i64>,
    /// When the window became `Complete` or `TimedOut`.
    finished_ms: Option<i64>,
}

impl WindowState {
    pub fn new(id: WindowId, session_id: &str, now_ms: i64) -> WindowState {
        let mut rec = WindowRecord {
            slug: id.slug(),
            asset: id.asset,
            start_ts: id.start,
            end_ts: id.end(),
            event_id: None,
            condition_id: None,
            up_token: None,
            down_token: None,
            price_to_beat_ours: None,
            final_twap_ours: None,
            outcome_ours: None,
            price_to_beat_gamma: None,
            final_price_gamma: None,
            twap_match: None,
            winner: None,
            resolved_at: None,
            resolution_seen_ms: None,
            twap_source: None,
            coverage_secs: 0,
            status: WindowStatus::Upcoming,
            session_id: session_id.to_string(),
            updated_ms: now_ms,
        };
        rec.status = status_at(&rec, now_ms);
        WindowState {
            id,
            rec,
            last_emitted: None,
            last_gamma_req_ms: None,
            last_resolution_req_ms: None,
            finished_ms: None,
        }
    }

    /// Stores Gamma's ids, tokens and prices. Prices only ever go from unknown to known.
    pub fn apply_gamma(&mut self, w: &GammaWindow) {
        let rec = &mut self.rec;
        rec.event_id = Some(w.event_id.clone());
        rec.condition_id = Some(w.condition_id.clone());
        rec.up_token = Some(w.up_token.clone());
        rec.down_token = Some(w.down_token.clone());
        rec.price_to_beat_gamma = w.price_to_beat.or(rec.price_to_beat_gamma);
        rec.final_price_gamma = w.final_price.or(rec.final_price_gamma);
    }

    /// Latches the winner: the first winner sticks. `resolved_at` and `resolution_seen_ms`
    /// are filled once.
    pub fn apply_winner(&mut self, winner: Outcome, resolved_at: Option<i64>, seen_ms: i64) {
        let rec = &mut self.rec;
        rec.winner.get_or_insert(winner);
        rec.resolved_at = rec.resolved_at.or(resolved_at);
        rec.resolution_seen_ms.get_or_insert(seen_ms);
    }

    /// Recomputes the derived fields (our price to beat and final TWAP, outcome, TWAP match)
    /// and the status at `now_ms`.
    pub fn refresh(&mut self, oracle: &OracleState, now_ms: i64) {
        let twap_at =
            |unix_s: i64| oracle.value_at(self.id.asset, OracleKind::Twap60, unix_s * 1000);
        let rec = &mut self.rec;
        // Re-read while the point is retained, so a PolyBolt value that arrives after the RTDS
        // one for the same timestamp still wins; once pruned, the last value is kept.
        if let Some((value, source)) = twap_at(self.id.start) {
            rec.price_to_beat_ours = Some(value);
            rec.twap_source = Some(source);
        }
        if let Some((value, _)) = twap_at(self.id.end()) {
            rec.final_twap_ours = Some(value);
        }
        if let (Some(ptb), Some(fin)) = (rec.price_to_beat_ours, rec.final_twap_ours) {
            // A tie resolves Up.
            rec.outcome_ours = Some(if fin >= ptb {
                Outcome::Up
            } else {
                Outcome::Down
            });
        }
        if let (Some(ours), Some(gamma)) = (rec.final_twap_ours, rec.final_price_gamma) {
            // Exact: Polymarket settles on this very f64.
            rec.twap_match = Some(ours == gamma);
        }
        rec.status = status_at(rec, now_ms);
        if matches!(rec.status, WindowStatus::Complete | WindowStatus::TimedOut) {
            self.finished_ms.get_or_insert(now_ms);
        }
    }

    /// The full record if anything other than `coverage_secs` changed since the last emission.
    /// The coverage counter rides along with the next real change (at the latest, the `Ended`
    /// transition), so a live window is not upserted every second.
    pub fn take_changed(&mut self, now_ms: i64) -> Option<WindowRecord> {
        let unchanged = self.last_emitted.as_ref().is_some_and(|last| {
            let mut cur = self.rec.clone();
            cur.coverage_secs = last.coverage_secs;
            cur.updated_ms = last.updated_ms;
            cur == *last
        });
        if unchanged {
            return None;
        }
        self.rec.updated_ms = now_ms;
        self.last_emitted = Some(self.rec.clone());
        Some(self.rec.clone())
    }

    /// The Gamma and resolution requests that are due at `now_ms`; marks them as sent.
    pub fn due_requests(&mut self, now_ms: i64) -> Vec<Request> {
        let end_ms = self.id.end() * 1000;
        if now_ms >= end_ms + GIVE_UP_MS {
            return vec![];
        }
        let rec = &self.rec;
        let since = |last: Option<i64>| last.map_or(i64::MAX, |t| now_ms - t);
        let mut out = vec![];

        let since_gamma = since(self.last_gamma_req_ms);
        let discovery = rec.condition_id.is_none() && since_gamma >= DISCOVERY_INTERVAL_MS;
        let final_price = now_ms >= end_ms + GAMMA_FINAL_START_MS
            && rec.final_price_gamma.is_none()
            && since_gamma >= GAMMA_FINAL_INTERVAL_MS;
        if discovery || final_price {
            out.push(Request::FetchGamma {
                slug: rec.slug.clone(),
            });
            self.last_gamma_req_ms = Some(now_ms);
        }

        if let Some(condition_id) = &rec.condition_id
            && rec.winner.is_none()
            && now_ms >= end_ms + RESOLUTION_START_MS
            && since(self.last_resolution_req_ms) >= RESOLUTION_INTERVAL_MS
        {
            out.push(Request::FetchResolution {
                slug: rec.slug.clone(),
                condition_id: condition_id.clone(),
            });
            self.last_resolution_req_ms = Some(now_ms);
        }
        out
    }

    /// Whether the CLOB should stay subscribed to this window's tokens.
    pub fn wants_tokens(&self) -> bool {
        matches!(
            self.rec.status,
            WindowStatus::Upcoming | WindowStatus::Live | WindowStatus::Ended
        )
    }

    pub fn tokens(&self) -> impl Iterator<Item = &String> {
        self.rec.up_token.iter().chain(self.rec.down_token.iter())
    }

    pub fn droppable(&self, now_ms: i64) -> bool {
        self.finished_ms
            .is_some_and(|t| now_ms - t >= DROP_AFTER_MS)
    }
}

fn status_at(rec: &WindowRecord, now_ms: i64) -> WindowStatus {
    let (start_ms, end_ms) = (rec.start_ts * 1000, rec.end_ts * 1000);
    if rec.winner.is_some() && rec.final_price_gamma.is_some() {
        WindowStatus::Complete
    } else if now_ms >= end_ms + GIVE_UP_MS {
        WindowStatus::TimedOut
    } else if rec.winner.is_some() {
        WindowStatus::Resolved
    } else if now_ms >= end_ms {
        WindowStatus::Ended
    } else if now_ms >= start_ms {
        WindowStatus::Live
    } else {
        WindowStatus::Upcoming
    }
}

#[cfg(test)]
mod tests {
    use raki_core::{Asset, Source};

    use super::*;

    const START: i64 = 1793031000;
    const START_MS: i64 = START * 1000;
    const END_MS: i64 = (START + 300) * 1000;

    fn window() -> WindowState {
        WindowState::new(
            WindowId {
                asset: Asset::Btc,
                start: START,
            },
            "session",
            0,
        )
    }

    #[test]
    fn status_follows_lifecycle() {
        let mut w = window();
        let oracle = OracleState::default();
        let at = |w: &mut WindowState, now| {
            w.refresh(&oracle, now);
            w.rec.status
        };
        assert_eq!(at(&mut w, START_MS - 1), WindowStatus::Upcoming);
        assert_eq!(at(&mut w, START_MS), WindowStatus::Live);
        assert_eq!(at(&mut w, END_MS - 1), WindowStatus::Live);
        assert_eq!(at(&mut w, END_MS), WindowStatus::Ended);
        w.apply_winner(Outcome::Up, None, END_MS + 30_000);
        assert_eq!(at(&mut w, END_MS + 30_000), WindowStatus::Resolved);
        assert_eq!(at(&mut w, END_MS + GIVE_UP_MS), WindowStatus::TimedOut);
        w.rec.final_price_gamma = Some(1.0);
        assert_eq!(at(&mut w, END_MS + GIVE_UP_MS), WindowStatus::Complete);
    }

    #[test]
    fn take_changed_ignores_coverage_only_changes() {
        let mut w = window();
        let mut oracle = OracleState::default();
        w.refresh(&oracle, START_MS);
        let first = w.take_changed(START_MS).expect("first emission");
        assert_eq!(first.updated_ms, START_MS);
        assert_eq!(first.session_id, "session");

        w.rec.coverage_secs += 1;
        assert_eq!(w.take_changed(START_MS + 1000), None);

        oracle.insert(
            Asset::Btc,
            OracleKind::Twap60,
            Source::Polybolt,
            START_MS,
            7.0,
        );
        w.refresh(&oracle, START_MS + 2000);
        let rec = w.take_changed(START_MS + 2000).expect("ptb changed");
        assert_eq!(rec.price_to_beat_ours, Some(7.0));
        assert_eq!(rec.coverage_secs, 1);
        assert_eq!(rec.updated_ms, START_MS + 2000);
    }
}

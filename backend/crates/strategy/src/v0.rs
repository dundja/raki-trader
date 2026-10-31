//! Strategy v0 (CLAUDE.md §3) as a shadow: late-window taker entry on the favourite when the model
//! beats the all-in price by `min_edge` (docs/specs/2026-10-31-shadow-predictions.md §4).

use raki_core::Outcome;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::fill::{Walk, walk_asks};

pub const STRATEGY_ID: &str = "v0";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct V0Params {
    /// Evaluate while `entry_window_end_s ≤ secs_left ≤ entry_window_start_s`.
    pub entry_window_start_s: i64,
    pub entry_window_end_s: i64,
    pub min_market_prob: Decimal,
    pub max_entry_price: Decimal,
    pub min_edge: Decimal,
    pub stake_usd: Decimal,
    /// From the decision to the book the simulated order fills against.
    pub sim_latency_ms: i64,
}

impl Default for V0Params {
    fn default() -> V0Params {
        V0Params {
            entry_window_start_s: 45,
            entry_window_end_s: 30,
            min_market_prob: Decimal::new(92, 2),
            max_entry_price: Decimal::new(97, 2),
            min_edge: Decimal::new(1, 2),
            stake_usd: Decimal::new(5, 0),
            sim_latency_ms: 250,
        }
    }
}

impl V0Params {
    pub fn in_entry_window(&self, secs_left: i64) -> bool {
        (self.entry_window_end_s..=self.entry_window_start_s).contains(&secs_left)
    }
}

/// The market parameters v0 depends on, read from the market's config at runtime.
#[derive(Clone, Debug, PartialEq)]
pub struct MarketConfig {
    pub fee_rate: Decimal,
    pub min_size: Decimal,
    pub tick: Decimal,
}

impl MarketConfig {
    /// From Gamma's `feeSchedule.rate` / `.exponent`, `orderMinSize` and
    /// `orderPriceMinTickSize`. Anything missing, or a fee exponent other than 1 (the formula v0
    /// knows), is an error naming the field.
    pub fn from_gamma(
        fee_rate: Option<f64>,
        fee_exponent: Option<f64>,
        min_size: Option<f64>,
        tick: Option<f64>,
    ) -> Result<MarketConfig, String> {
        let dec = |name: &str, v: Option<f64>| {
            v.and_then(|v| v.to_string().parse::<Decimal>().ok())
                .ok_or_else(|| format!("{name} missing"))
        };
        let exponent = dec("fee exponent", fee_exponent)?;
        if exponent != Decimal::ONE {
            return Err(format!("fee exponent {exponent}, expected 1"));
        }
        Ok(MarketConfig {
            fee_rate: dec("fee rate", fee_rate)?,
            min_size: dec("min order size", min_size)?,
            tick: dec("tick size", tick)?,
        })
    }
}

/// One side's book as v0 sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SideBook {
    pub best_bid: Option<Decimal>,
    /// (price, size), ascending by price.
    pub asks: Vec<(Decimal, Decimal)>,
}

impl SideBook {
    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.first().map(|(p, _)| *p)
    }

    /// The mid when both sides exist, else the best bid (a favourite bid at 0.99 with no asks
    /// is still the favourite; there is just nothing to buy).
    pub fn market_prob(&self) -> Option<Decimal> {
        match (self.best_bid, self.best_ask()) {
            (Some(b), Some(a)) => Some((b + a) / Decimal::TWO),
            (Some(b), None) => Some(b),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EvalInput {
    /// Control is `running` (which implies every safety check passed at the last tick).
    pub running: bool,
    pub p_up: Option<f64>,
    pub config: Result<MarketConfig, String>,
    pub up: Option<SideBook>,
    pub down: Option<SideBook>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    NotRunning,
    NoPrediction,
    MarketConfig,
    NoBook,
    NoFavourite,
    NoLiquidity,
    Edge,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::NotRunning => "not_running",
            SkipReason::NoPrediction => "no_prediction",
            SkipReason::MarketConfig => "market_config",
            SkipReason::NoBook => "no_book",
            SkipReason::NoFavourite => "no_favourite",
            SkipReason::NoLiquidity => "no_liquidity",
            SkipReason::Edge => "edge",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// A FAK buy of `side` for the stake, at most `limit` per share.
    Enter { limit: Decimal },
    Skip(SkipReason),
}

/// One evaluation, with whatever was known when it stopped.
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    pub side: Option<Outcome>,
    pub model_prob: Option<f64>,
    pub market_prob: Option<Decimal>,
    pub best_ask: Option<Decimal>,
    pub walk: Option<Walk>,
    pub edge: Option<Decimal>,
    pub decision: Decision,
}

impl Evaluation {
    fn skip(reason: SkipReason) -> Evaluation {
        Evaluation {
            side: None,
            model_prob: None,
            market_prob: None,
            best_ask: None,
            walk: None,
            edge: None,
            decision: Decision::Skip(reason),
        }
    }
}

/// Runs the v0 gates in order: running, prediction, market config, books, favourite, liquidity at
/// or below `max_entry_price` (at least the market's min size), edge.
pub fn evaluate(p: &V0Params, input: &EvalInput) -> Evaluation {
    if !input.running {
        return Evaluation::skip(SkipReason::NotRunning);
    }
    let Some(p_up) = input.p_up else {
        return Evaluation::skip(SkipReason::NoPrediction);
    };
    let Ok(config) = &input.config else {
        return Evaluation::skip(SkipReason::MarketConfig);
    };
    let (Some(up), Some(down)) = (&input.up, &input.down) else {
        return Evaluation::skip(SkipReason::NoBook);
    };

    let fav = |side: Outcome, book: &SideBook| {
        book.market_prob()
            .filter(|m| *m >= p.min_market_prob)
            .map(|m| (side, m))
    };
    let favourite = match (fav(Outcome::Up, up), fav(Outcome::Down, down)) {
        (Some(u), Some(d)) => Some(if d.1 > u.1 { d } else { u }),
        (u, d) => u.or(d),
    };
    let Some((side, market_prob)) = favourite else {
        return Evaluation::skip(SkipReason::NoFavourite);
    };
    let book = if side == Outcome::Up { up } else { down };
    let model_prob = if side == Outcome::Up { p_up } else { 1.0 - p_up };
    let mut ev = Evaluation {
        side: Some(side),
        model_prob: Some(model_prob),
        market_prob: Some(market_prob),
        best_ask: book.best_ask(),
        walk: None,
        edge: None,
        decision: Decision::Skip(SkipReason::NoLiquidity),
    };

    let walk = walk_asks(
        book.asks.iter().copied(),
        p.max_entry_price,
        p.stake_usd,
        config.fee_rate,
    );
    let (Some(all_in), Some(limit)) = (walk.all_in_per_share(), walk.worst_price) else {
        ev.walk = Some(walk);
        return ev;
    };
    if walk.shares < config.min_size {
        ev.walk = Some(walk);
        return ev;
    }
    // f64 → Decimal through its shortest round-trip text, like every other oracle value.
    let edge = model_prob
        .to_string()
        .parse::<Decimal>()
        .ok()
        .map(|m| m - all_in);
    ev.walk = Some(walk);
    ev.edge = edge;
    ev.decision = match edge {
        Some(e) if e >= p.min_edge => Decision::Enter { limit },
        _ => Decision::Skip(SkipReason::Edge),
    };
    ev
}

#[derive(Clone, Debug, PartialEq)]
pub struct Intent {
    /// The evaluation that made it.
    pub seq: u64,
    pub side: Outcome,
    pub limit: Decimal,
    pub stake: Decimal,
    pub fee_rate: Decimal,
    pub decided_ms: i64,
    pub due_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FillStatus {
    Filled,
    Partial,
    NoFill,
}

impl FillStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FillStatus::Filled => "filled",
            FillStatus::Partial => "partial",
            FillStatus::NoFill => "no_fill",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    pub intent: Intent,
    pub filled_ms: i64,
    pub walk: Walk,
    pub status: FillStatus,
}

/// v0 in one window: evaluations until an intent, its simulated fill at the due time, and at
/// most one position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct V0Window {
    pending: Option<Intent>,
    filled: bool,
}

impl V0Window {
    /// Whether evaluating now is pointless: a fill happened, or an intent awaits its fill.
    pub fn idle(&self) -> bool {
        self.filled || self.pending.is_some()
    }

    pub fn pending(&self) -> Option<&Intent> {
        self.pending.as_ref()
    }

    /// Evaluates at `now_ms` unless [`idle`](Self::idle); an `Enter` becomes the pending intent.
    pub fn evaluate(
        &mut self,
        p: &V0Params,
        input: &EvalInput,
        seq: u64,
        now_ms: i64,
    ) -> Option<Evaluation> {
        if self.idle() {
            return None;
        }
        let ev = evaluate(p, input);
        if let (Decision::Enter { limit }, Some(side), Ok(config)) =
            (&ev.decision, ev.side, &input.config)
        {
            self.pending = Some(Intent {
                seq,
                side,
                limit: *limit,
                stake: p.stake_usd,
                fee_rate: config.fee_rate,
                decided_ms: now_ms,
                due_ms: now_ms + p.sim_latency_ms,
            });
        }
        Some(ev)
    }

    /// Fills the pending intent against `asks` (the intent's side, as the book is at `now_ms`)
    /// once it is due. A fill ends the window; a no-fill lets a later tick try again.
    pub fn fill_due(&mut self, now_ms: i64, asks: &[(Decimal, Decimal)]) -> Option<Fill> {
        if self.pending.as_ref()?.due_ms > now_ms {
            return None;
        }
        let intent = self.pending.take()?;
        let walk = walk_asks(asks.iter().copied(), intent.limit, intent.stake, intent.fee_rate);
        let status = if walk.shares.is_zero() {
            FillStatus::NoFill
        } else if walk.stake_spent {
            FillStatus::Filled
        } else {
            FillStatus::Partial
        };
        self.filled = status != FillStatus::NoFill;
        Some(Fill {
            intent,
            filled_ms: now_ms,
            walk,
            status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn side(bid: &str, asks: &[(&str, &str)]) -> Option<SideBook> {
        Some(SideBook {
            best_bid: Some(d(bid)),
            asks: asks.iter().map(|(p, s)| (d(p), d(s))).collect(),
        })
    }

    fn config() -> MarketConfig {
        MarketConfig::from_gamma(Some(0.07), Some(1.0), Some(5.0), Some(0.01)).unwrap()
    }

    /// Up is the favourite: bid 0.93, asks 0.94 (100); the model says 0.98.
    fn input() -> EvalInput {
        EvalInput {
            running: true,
            p_up: Some(0.98),
            config: Ok(config()),
            up: side("0.93", &[("0.94", "100")]),
            down: side("0.05", &[("0.07", "100")]),
        }
    }

    /// Equal up to the rounding of a 28-digit share count.
    fn close(got: Option<Decimal>, want: &str) {
        let diff = (got.expect("a value") - d(want)).abs();
        assert!(diff < d("1e-18"), "{got:?} vs {want}");
    }

    fn reason(ev: &Evaluation) -> Option<SkipReason> {
        match ev.decision {
            Decision::Skip(r) => Some(r),
            Decision::Enter { .. } => None,
        }
    }

    #[test]
    fn market_config_from_gamma() {
        assert_eq!(
            config(),
            MarketConfig {
                fee_rate: d("0.07"),
                min_size: d("5"),
                tick: d("0.01"),
            }
        );
        let err = MarketConfig::from_gamma(Some(0.07), Some(2.0), Some(5.0), Some(0.01));
        assert_eq!(err, Err("fee exponent 2, expected 1".into()));
        let err = MarketConfig::from_gamma(None, Some(1.0), Some(5.0), Some(0.01));
        assert_eq!(err, Err("fee rate missing".into()));
    }

    #[test]
    fn enters_the_favourite_when_the_edge_clears() {
        let ev = evaluate(&V0Params::default(), &input());
        assert_eq!(ev.side, Some(Outcome::Up));
        assert_eq!(ev.market_prob, Some(d("0.935")));
        assert_eq!(ev.best_ask, Some(d("0.94")));
        let walk = ev.walk.as_ref().unwrap();
        assert_eq!(walk.shares, d("5") / d("0.94"));
        // 0.98 − (0.94 + 0.07 · 0.94 · 0.06) = 0.98 − 0.943948 = 0.036052.
        close(ev.edge, "0.036052");
        assert_eq!(ev.decision, Decision::Enter { limit: d("0.94") });
    }

    #[test]
    fn down_side_uses_one_minus_p_up() {
        let mut i = input();
        i.p_up = Some(0.03);
        i.up = side("0.05", &[("0.07", "100")]);
        i.down = side("0.93", &[("0.95", "100")]);
        let ev = evaluate(&V0Params::default(), &i);
        assert_eq!(ev.side, Some(Outcome::Down));
        assert_eq!(ev.model_prob, Some(0.97));
        // 0.97 − (0.95 + 0.07 · 0.95 · 0.05) = 0.016675.
        close(ev.edge, "0.016675");
        assert!(matches!(ev.decision, Decision::Enter { .. }));
    }

    #[test]
    fn each_gate_has_its_skip_reason() {
        let p = V0Params::default();
        let check = |f: &dyn Fn(&mut EvalInput), want: SkipReason| {
            let mut i = input();
            f(&mut i);
            assert_eq!(reason(&evaluate(&p, &i)), Some(want));
        };
        check(&|i| i.running = false, SkipReason::NotRunning);
        check(&|i| i.p_up = None, SkipReason::NoPrediction);
        check(&|i| i.config = Err("x".into()), SkipReason::MarketConfig);
        check(&|i| i.down = None, SkipReason::NoBook);
        // Mid 0.915 is under 0.92.
        check(&|i| i.up = side("0.91", &[("0.92", "100")]), SkipReason::NoFavourite);
        // The favourite (bid 0.99) has no asks: nothing to buy.
        check(&|i| i.up = side("0.99", &[]), SkipReason::NoLiquidity);
        // Asks only above max_entry_price.
        check(&|i| i.up = side("0.97", &[("0.98", "100")]), SkipReason::NoLiquidity);
        // 4 shares at or below 0.97 is under the 5-share minimum.
        check(
            &|i| i.up = side("0.93", &[("0.94", "4"), ("0.99", "100")]),
            SkipReason::NoLiquidity,
        );
        // 0.95 − 0.943948 = 0.006052 < 0.01.
        check(&|i| i.p_up = Some(0.95), SkipReason::Edge);
    }

    #[test]
    fn the_skip_keeps_what_was_known() {
        let mut i = input();
        i.p_up = Some(0.95);
        let ev = evaluate(&V0Params::default(), &i);
        close(ev.edge, "0.006052");
        assert!(ev.walk.is_some());
    }

    #[test]
    fn entry_window_is_inclusive() {
        let p = V0Params::default();
        assert!(!p.in_entry_window(46));
        assert!(p.in_entry_window(45));
        assert!(p.in_entry_window(30));
        assert!(!p.in_entry_window(29));
    }

    #[test]
    fn fills_against_the_book_at_the_due_time_once_per_window() {
        let p = V0Params::default();
        let mut w = V0Window::default();
        let ev = w.evaluate(&p, &input(), 7, 10_000).unwrap();
        assert!(matches!(ev.decision, Decision::Enter { .. }));
        assert_eq!(w.pending().unwrap().due_ms, 10_250);
        assert!(w.idle());
        assert_eq!(w.evaluate(&p, &input(), 8, 10_100), None, "pending: no evaluation");

        let later = [(d("0.94"), d("2")), (d("0.96"), d("100"))];
        assert_eq!(w.fill_due(10_249, &later), None, "not due yet");
        // At the due time only 2 shares are left at the 0.94 limit: a partial fill.
        let fill = w.fill_due(10_250, &later).unwrap();
        assert_eq!(fill.intent.seq, 7);
        assert_eq!(fill.status, FillStatus::Partial);
        assert_eq!((fill.walk.shares, fill.walk.cost), (d("2"), d("1.88")));
        assert!(w.idle(), "one position per window");
        assert_eq!(w.evaluate(&p, &input(), 9, 11_000), None);
    }

    #[test]
    fn a_no_fill_allows_another_try() {
        let p = V0Params::default();
        let mut w = V0Window::default();
        w.evaluate(&p, &input(), 1, 0).unwrap();
        let moved = [(d("0.95"), d("100"))];
        let fill = w.fill_due(300, &moved).unwrap();
        assert_eq!(fill.status, FillStatus::NoFill);
        assert!(!w.idle());
        w.evaluate(&p, &input(), 2, 1_000).unwrap();
        let fill = w.fill_due(1_250, &[(d("0.94"), d("100"))]).unwrap();
        assert_eq!(fill.status, FillStatus::Filled);
        assert_eq!(fill.walk.cost, d("5"));
        assert_eq!(fill.intent.seq, 2);
    }
}

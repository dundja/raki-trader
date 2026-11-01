//! The engine: consumes [`FeedEvent`]s and a 1 s tick, and emits [`EngineOutput`]s for the shell.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use raki_core::{
    Asset, FeedEvent, LiveWindow, Outcome, PredictionRow, SeriesRow, ShadowEvalRow, ShadowFillRow,
    Source, TradingMode, WindowId, WindowRecord, WindowStatus,
};

use crate::book::BookState;
use crate::cex::CexState;
use crate::control::Control;
use crate::oracle::OracleState;
use crate::series::WindowPrices;
use crate::shadow::{self, ShadowConfig};
use crate::windows::WindowState;

/// A gap this long between two calls means tracking paused (the machine slept, or the loop was
/// stalled): every window then gets a resume grace before it can time out.
pub const RESUME_AFTER_GAP_MS: i64 = 10_000;

/// The CLOB tokens to subscribe to, per asset (one CLOB connection per asset). Every configured
/// asset has an entry, possibly empty.
pub type DesiredTokens = BTreeMap<Asset, BTreeSet<String>>;

/// A REST call the shell runs; its result comes back as a [`FeedEvent`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    FetchGamma { slug: String },
    FetchResolution { slug: String, condition_id: String },
}

// The brief fixes `Window(WindowRecord)` unboxed; outputs are few per second and short-lived.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum EngineOutput {
    Series(Vec<SeriesRow>),
    /// Upsert of the full current record.
    Window(WindowRecord),
    Requests(Vec<Request>),
    /// The CLOB tokens to stay subscribed to, per asset; emitted when any set changes.
    DesiredTokens(DesiredTokens),
    /// A book snapshot disagreed with our delta-built book.
    BookResync {
        asset_id: String,
    },
    /// Shadow (no orders, never a gate): a model forecast at a checkpoint, a strategy v0
    /// evaluation, and the simulated fill of an `enter`.
    Prediction(PredictionRow),
    ShadowEval(ShadowEvalRow),
    ShadowFill(ShadowFillRow),
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub assets: Vec<Asset>,
    pub session_id: String,
    /// `None` turns predictions and the v0 shadow off.
    pub shadow: Option<ShadowConfig>,
}

#[derive(Debug)]
pub struct Engine {
    cfg: EngineConfig,
    books: HashMap<String, BookState>,
    oracle: OracleState,
    cex: CexState,
    windows: BTreeMap<WindowId, WindowState>,
    control: Control,
    book_resyncs: u32,
    desired: DesiredTokens,
    /// The latest `now_ms` seen by `handle` or `tick`, to notice a pause.
    last_seen_ms: Option<i64>,
    params_hash: String,
    /// Next shadow evaluation number (unique per session).
    eval_seq: u64,
}

impl Engine {
    pub fn new(cfg: EngineConfig, now_ms: i64) -> Engine {
        let params_hash = cfg
            .shadow
            .as_ref()
            .map(ShadowConfig::params_hash)
            .unwrap_or_default();
        Engine {
            cfg,
            params_hash,
            eval_seq: 0,
            books: HashMap::new(),
            oracle: OracleState::default(),
            cex: CexState::default(),
            windows: BTreeMap::new(),
            control: Control::new(now_ms),
            book_resyncs: 0,
            desired: DesiredTokens::new(),
            last_seen_ms: None,
        }
    }

    /// Resumes tracking a window an earlier session left unfinished (from the store). Its latched
    /// fields are kept, and it gets at least the resume grace of polling. Ignored for an asset
    /// that is not configured, a malformed slug, or a window already tracked.
    pub fn adopt(&mut self, rec: WindowRecord, now_ms: i64) {
        let Some(id) = WindowId::parse_slug(&rec.slug) else {
            tracing::warn!(slug = %rec.slug, "cannot adopt a window with a malformed slug");
            return;
        };
        if !self.cfg.assets.contains(&id.asset) {
            return;
        }
        self.windows
            .entry(id)
            .or_insert_with(|| WindowState::adopt(id, rec, now_ms));
    }

    /// Every tracked window's current record, for a final write at shutdown (it carries the
    /// coverage counted since the last upsert).
    pub fn records(&self) -> Vec<WindowRecord> {
        self.windows.values().map(|w| w.rec.clone()).collect()
    }

    /// Notices a pause (sleep, stall): every window then gets a resume grace.
    fn observe(&mut self, now_ms: i64) {
        if let Some(prev) = self.last_seen_ms
            && now_ms - prev > RESUME_AFTER_GAP_MS
        {
            tracing::info!(
                paused_ms = now_ms - prev,
                "tracking resumed after a pause; windows get a polling grace"
            );
            for w in self.windows.values_mut() {
                w.resume(now_ms);
            }
        }
        self.last_seen_ms = Some(self.last_seen_ms.map_or(now_ms, |p| p.max(now_ms)));
    }

    pub fn handle(&mut self, ev: FeedEvent, now_ms: i64) -> Vec<EngineOutput> {
        self.observe(now_ms);
        let mut out = vec![];
        // A simulated fill sees the book as it was at its due time: before the first book
        // event at or after it.
        if matches!(ev, FeedEvent::Book { .. } | FeedEvent::PriceChange { .. }) {
            self.fill_due(now_ms, &mut out);
        }
        match ev {
            FeedEvent::Book {
                asset_id,
                bids,
                asks,
                ..
            } => {
                let had_state = self.books.contains_key(&asset_id);
                let book = self.books.entry(asset_id.clone()).or_default();
                if book.apply_snapshot(&bids, &asks) && had_state {
                    self.book_resyncs += 1;
                    out.push(EngineOutput::BookResync { asset_id });
                }
            }
            FeedEvent::PriceChange { changes, .. } => {
                for c in changes {
                    // A delta without a base snapshot cannot build a correct book; the CLOB
                    // always sends the snapshot first after (re)subscribing.
                    if let Some(book) = self.books.get_mut(&c.asset_id) {
                        book.apply_change(c.side, c.price, c.size);
                    }
                }
            }
            // A reconnect's books are stale until its fresh snapshots arrive. A shard's reset
            // clears only its own asset's books: the other shards' deltas keep flowing.
            FeedEvent::FeedReset {
                source: Source::ClobMarket,
                shard,
            } => match shard {
                None => self.books.clear(),
                Some(asset) => {
                    let tokens: BTreeSet<&String> = self
                        .windows
                        .values()
                        .filter(|w| w.id.asset == asset)
                        .flat_map(|w| w.tokens())
                        .collect();
                    self.books.retain(|id, _| !tokens.contains(id));
                }
            },
            FeedEvent::Oracle {
                asset,
                kind,
                source,
                ts_ms,
                value,
            } => {
                self.oracle.insert(asset, kind, source, ts_ms, value);
                self.sync(now_ms, &mut out);
            }
            FeedEvent::CexQuote {
                venue,
                asset,
                bid,
                ask,
                recv_ms,
            } => self.cex.on_quote(venue, asset, bid, ask, recv_ms),
            FeedEvent::UsdtUsd { price, recv_ms } => self.cex.on_usdt(price, recv_ms),
            FeedEvent::GammaFetched {
                slug,
                window: Some(gamma),
                ..
            } => {
                if let Some(w) =
                    WindowId::parse_slug(&slug).and_then(|id| self.windows.get_mut(&id))
                {
                    w.apply_gamma(&gamma);
                    self.sync(now_ms, &mut out);
                }
            }
            FeedEvent::MarketResolved {
                condition_id,
                winning_asset_id,
                winning_outcome,
                recv_ms,
            } => {
                if let Some(w) = self.window_by_condition(&condition_id) {
                    let rec = &w.rec;
                    let winner = if rec.up_token.as_ref() == Some(&winning_asset_id) {
                        Some(Outcome::Up)
                    } else if rec.down_token.as_ref() == Some(&winning_asset_id) {
                        Some(Outcome::Down)
                    } else {
                        match winning_outcome.as_str() {
                            "Up" => Some(Outcome::Up),
                            "Down" => Some(Outcome::Down),
                            _ => None,
                        }
                    };
                    match winner {
                        Some(winner) => {
                            w.apply_winner(winner, None, recv_ms);
                            self.sync(now_ms, &mut out);
                        }
                        None => tracing::warn!(
                            slug = %rec.slug,
                            %winning_asset_id,
                            %winning_outcome,
                            "market_resolved winner matches neither token nor outcome label"
                        ),
                    }
                }
            }
            FeedEvent::Resolution {
                condition_id,
                winner: Some(winner),
                resolved_at,
            } => {
                if let Some(w) = self.window_by_condition(&condition_id) {
                    w.apply_winner(winner, resolved_at, now_ms);
                    self.sync(now_ms, &mut out);
                }
            }
            // Recorded raw; nothing to track in M1. A 404, a failed fetch or an unresolved
            // market is retried on the window's own schedule.
            FeedEvent::BestBidAsk { .. }
            | FeedEvent::LastTrade { .. }
            | FeedEvent::TickSize { .. }
            | FeedEvent::FeedReset { .. }
            | FeedEvent::SeqGap { .. }
            | FeedEvent::GammaFetched { window: None, .. }
            | FeedEvent::GammaFailed { .. }
            | FeedEvent::Resolution { winner: None, .. } => {}
        }
        out
    }

    /// Called once per second.
    pub fn tick(&mut self, now_ms: i64) -> Vec<EngineOutput> {
        self.observe(now_ms);
        let now_s = now_ms.div_euclid(1000);
        for &asset in &self.cfg.assets {
            let current = WindowId::containing(asset, now_s);
            for id in [current, current.next()] {
                self.windows
                    .entry(id)
                    .or_insert_with(|| WindowState::new(id, &self.cfg.session_id, now_ms));
            }
        }
        self.oracle.prune(now_ms);

        let mut out = vec![];
        self.sync(now_ms, &mut out);

        let mut rows = vec![];
        for w in self.windows.values_mut() {
            if w.rec.status == WindowStatus::Live {
                w.rec.coverage_secs += 1;
                let prices = prices(&self.oracle, &self.cex, &self.books, &w.rec);
                rows.push(prices.series_row(&w.rec.slug, now_s));
            }
        }
        if !rows.is_empty() {
            out.push(EngineOutput::Series(rows));
        }
        self.shadow_tick(now_ms, &mut out);

        let requests: Vec<Request> = self
            .windows
            .values_mut()
            .flat_map(|w| w.due_requests(now_ms))
            .collect();
        if !requests.is_empty() {
            out.push(EngineOutput::Requests(requests));
        }
        out
    }

    /// The current window of every configured asset, with the latest prices.
    pub fn live_windows(&self, now_ms: i64) -> Vec<LiveWindow> {
        self.cfg
            .assets
            .iter()
            .map(|&asset| {
                let id = WindowId::containing(asset, now_ms.div_euclid(1000));
                let rec = self.windows.get(&id).map(|w| &w.rec);
                let p = match rec {
                    Some(rec) => prices(&self.oracle, &self.cex, &self.books, rec),
                    None => WindowPrices::collect(
                        asset,
                        None,
                        None,
                        &self.oracle,
                        &self.cex,
                        &self.books,
                    ),
                };
                LiveWindow {
                    asset,
                    slug: id.slug(),
                    start_ts: id.start,
                    end_ts: id.end(),
                    secs_left: id.secs_left(now_ms),
                    price_to_beat: rec.and_then(|r| r.price_to_beat_ours.or(r.price_to_beat_gamma)),
                    twap: p.twap.map(|(_, value)| value),
                    twap_ts_ms: p.twap.map(|(ts, _)| ts),
                    chainlink: p.chainlink,
                    binance_mid_usd: p.binance_mid_usd,
                    coinbase_mid: p.coinbase_mid,
                    up_bid: p.up_bid,
                    up_ask: p.up_ask,
                    down_bid: p.down_bid,
                    down_ask: p.down_ask,
                    prediction: self
                        .windows
                        .get(&id)
                        .and_then(|w| w.shadow.live_prediction()),
                    shadow: self
                        .cfg
                        .shadow
                        .as_ref()
                        .filter(|s| s.v0_assets.contains(&asset))
                        .map(|_| {
                            self.windows
                                .get(&id)
                                .map(|w| w.shadow.live_shadow())
                                .unwrap_or_default()
                        }),
                }
            })
            .collect()
    }

    /// The shadow parameter set's hash (empty with the shadow off).
    pub fn params_hash(&self) -> &str {
        &self.params_hash
    }

    pub fn shadow_config(&self) -> Option<&ShadowConfig> {
        self.cfg.shadow.as_ref()
    }

    fn shadow_tick(&mut self, now_ms: i64, out: &mut Vec<EngineOutput>) {
        let Some(cfg) = &self.cfg.shadow else {
            return;
        };
        let ctx = shadow::Ctx {
            cfg,
            params_hash: &self.params_hash,
            session_id: &self.cfg.session_id,
            oracle: &self.oracle,
            cex: &self.cex,
            books: &self.books,
            safe: self.control.safety_ok(),
            control_running: self.control.state().mode == TradingMode::Running,
        };
        for w in self.windows.values_mut() {
            shadow::on_tick(&ctx, w, now_ms, &mut self.eval_seq, out);
        }
    }

    fn fill_due(&mut self, now_ms: i64, out: &mut Vec<EngineOutput>) {
        for w in self.windows.values_mut() {
            shadow::fill_due(&self.cfg.session_id, &self.books, w, now_ms, out);
        }
    }

    pub fn control(&mut self) -> &mut Control {
        &mut self.control
    }

    pub fn book_resyncs(&self) -> u32 {
        self.book_resyncs
    }

    fn window_by_condition(&mut self, condition_id: &str) -> Option<&mut WindowState> {
        self.windows
            .values_mut()
            .find(|w| w.rec.condition_id.as_deref() == Some(condition_id))
    }

    /// Recomputes every window, emits the changed records, forgets finished windows and emits
    /// the desired token set if it changed (dropping the books of tokens no longer wanted).
    fn sync(&mut self, now_ms: i64, out: &mut Vec<EngineOutput>) {
        for w in self.windows.values_mut() {
            w.refresh(&self.oracle, now_ms);
            if let Some(rec) = w.take_changed(now_ms) {
                out.push(EngineOutput::Window(rec));
            }
        }
        self.windows.retain(|_, w| !w.droppable(now_ms));

        let mut desired: DesiredTokens = self
            .cfg
            .assets
            .iter()
            .map(|&a| (a, BTreeSet::new()))
            .collect();
        for w in self.windows.values().filter(|w| w.wants_tokens()) {
            if let Some(set) = desired.get_mut(&w.id.asset) {
                set.extend(w.tokens().cloned());
            }
        }
        if desired != self.desired {
            // An unsubscribed token's book would only go stale.
            self.books
                .retain(|asset_id, _| desired.values().any(|set| set.contains(asset_id)));
            self.desired = desired.clone();
            out.push(EngineOutput::DesiredTokens(desired));
        }
    }
}

fn prices(
    oracle: &OracleState,
    cex: &CexState,
    books: &HashMap<String, BookState>,
    rec: &WindowRecord,
) -> WindowPrices {
    WindowPrices::collect(
        rec.asset,
        rec.up_token.as_deref(),
        rec.down_token.as_deref(),
        oracle,
        cex,
        books,
    )
}

#[cfg(test)]
mod tests {
    use raki_core::{
        GammaWindow, Level, OracleKind, Outcome, PriceChangeEntry, Side, Source, Venue,
        WindowStatus,
    };
    use rust_decimal::Decimal;

    use super::*;
    use crate::windows::{GIVE_UP_MS, RESUME_GRACE_MS};

    const START: i64 = 1793031000;
    const NEXT: i64 = START + 300;
    const START_MS: i64 = START * 1000;
    const END_MS: i64 = NEXT * 1000;

    fn engine() -> Engine {
        Engine::new(
            EngineConfig {
                assets: vec![Asset::Btc],
                session_id: "session-1".into(),
                shadow: None,
            },
            START_MS,
        )
    }

    fn slug(start: i64) -> String {
        format!("btc-updown-5m-{start}")
    }

    fn up(start: i64) -> String {
        format!("up-{start}")
    }

    fn down(start: i64) -> String {
        format!("down-{start}")
    }

    fn cond(start: i64) -> String {
        format!("cond-{start}")
    }

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn gamma(start: i64, final_price: Option<f64>) -> FeedEvent {
        FeedEvent::GammaFetched {
            slug: slug(start),
            window: Some(GammaWindow {
                slug: slug(start),
                event_id: format!("ev-{start}"),
                start,
                end: start + 300,
                condition_id: cond(start),
                up_token: up(start),
                down_token: down(start),
                price_to_beat: Some(99.5),
                final_price,
                closed: final_price.is_some(),
                twap_lookback_s: Some(60),
                fee_rate: Some(0.07),
                fee_exponent: Some(1.0),
                tick_size: Some(0.01),
                order_min_size: Some(5.0),
            }),
            recv_ms: 0,
        }
    }

    fn twap(ts_ms: i64, value: f64) -> FeedEvent {
        FeedEvent::Oracle {
            asset: Asset::Btc,
            kind: OracleKind::Twap60,
            source: Source::RtdsLegacy,
            ts_ms,
            value,
        }
    }

    fn book(asset_id: &str, bid: &str, ask: &str) -> FeedEvent {
        FeedEvent::Book {
            asset_id: asset_id.into(),
            market: "m".into(),
            ts_ms: 0,
            bids: vec![Level {
                price: d(bid),
                size: d("10"),
            }],
            asks: vec![Level {
                price: d(ask),
                size: d("10"),
            }],
        }
    }

    fn requests(out: &[EngineOutput]) -> Vec<Request> {
        out.iter()
            .flat_map(|o| match o {
                EngineOutput::Requests(r) => r.clone(),
                _ => vec![],
            })
            .collect()
    }

    fn rows(out: &[EngineOutput], start: i64) -> Vec<SeriesRow> {
        out.iter()
            .flat_map(|o| match o {
                EngineOutput::Series(r) => r.clone(),
                _ => vec![],
            })
            .filter(|r| r.slug == slug(start))
            .collect()
    }

    fn window_outputs(out: &[EngineOutput], start: i64) -> Vec<WindowRecord> {
        out.iter()
            .filter_map(|o| match o {
                EngineOutput::Window(w) if w.slug == slug(start) => Some(w.clone()),
                _ => None,
            })
            .collect()
    }

    /// The BTC set of the latest desired-tokens output.
    fn desired(out: &[EngineOutput]) -> Option<BTreeSet<String>> {
        out.iter().rev().find_map(|o| match o {
            EngineOutput::DesiredTokens(t) => Some(t[&Asset::Btc].clone()),
            _ => None,
        })
    }

    fn record(e: &Engine, start: i64) -> WindowRecord {
        e.windows[&WindowId {
            asset: Asset::Btc,
            start,
        }]
            .rec
            .clone()
    }

    fn fetch_gamma(start: i64) -> Request {
        Request::FetchGamma { slug: slug(start) }
    }

    fn fetch_resolution(start: i64) -> Request {
        Request::FetchResolution {
            slug: slug(start),
            condition_id: cond(start),
        }
    }

    /// An engine past discovery: both windows known from Gamma.
    fn discovered(final_price: Option<f64>) -> Engine {
        let mut e = engine();
        e.tick(START_MS + 1000);
        e.handle(gamma(START, final_price), START_MS + 1100);
        e.handle(gamma(NEXT, None), START_MS + 1200);
        e
    }

    #[test]
    fn discovery_requests_current_and_next() {
        let mut e = engine();
        let reqs = requests(&e.tick(START_MS + 1000));
        assert!(reqs.contains(&fetch_gamma(START)), "{reqs:?}");
        assert!(reqs.contains(&fetch_gamma(NEXT)), "{reqs:?}");

        assert_eq!(requests(&e.tick(START_MS + 2000)), vec![]);
        assert_eq!(requests(&e.tick(START_MS + 5999)), vec![]);
        let again = requests(&e.tick(START_MS + 6000));
        assert!(
            again.contains(&fetch_gamma(START)),
            "still no condition_id: {again:?}"
        );
    }

    #[test]
    fn gamma_fetched_sets_desired_tokens() {
        let mut e = engine();
        e.tick(START_MS + 1000);

        let out = e.handle(gamma(START, None), START_MS + 1100);
        let w = window_outputs(&out, START).pop().expect("window upsert");
        assert_eq!(w.condition_id, Some(cond(START)));
        assert_eq!(w.event_id, Some(format!("ev-{START}")));
        assert_eq!(w.up_token, Some(up(START)));
        assert_eq!(w.down_token, Some(down(START)));
        assert_eq!(w.price_to_beat_gamma, Some(99.5));
        assert_eq!(w.status, WindowStatus::Live);
        assert_eq!(w.session_id, "session-1");
        assert_eq!(w.updated_ms, START_MS + 1100);
        assert_eq!(
            desired(&out),
            Some(BTreeSet::from([up(START), down(START)]))
        );

        let out = e.handle(gamma(NEXT, None), START_MS + 1200);
        assert_eq!(
            desired(&out),
            Some(BTreeSet::from([
                up(START),
                down(START),
                up(NEXT),
                down(NEXT)
            ]))
        );

        let out = e.handle(gamma(NEXT, None), START_MS + 1300);
        assert_eq!(desired(&out), None, "unchanged set is not re-emitted");
        assert!(window_outputs(&out, NEXT).is_empty(), "unchanged record");

        let reqs = requests(&e.tick(START_MS + 7000));
        assert!(!reqs.contains(&fetch_gamma(START)), "{reqs:?}");
    }

    #[test]
    fn ours_ptb_and_final_from_twap_points_and_tie_is_up() {
        let mut e = discovered(None);
        e.handle(twap(START_MS - 1000, 98.0), START_MS + 1300);
        let out = e.handle(twap(START_MS, 100.0), START_MS + 1400);
        let w = window_outputs(&out, START).pop().expect("ptb emitted");
        assert_eq!(w.price_to_beat_ours, Some(100.0));
        assert_eq!(w.twap_source, Some(Source::RtdsLegacy));
        assert_eq!(w.final_twap_ours, None);
        assert_eq!(w.outcome_ours, None);

        e.handle(twap(END_MS - 1000, 101.0), END_MS - 500);
        let out = e.handle(twap(END_MS, 100.0), END_MS + 100);
        let w = window_outputs(&out, START).pop().expect("final emitted");
        assert_eq!(w.final_twap_ours, Some(100.0));
        assert_eq!(w.outcome_ours, Some(Outcome::Up));
        assert_eq!(w.twap_source, Some(Source::RtdsLegacy));
        // The final point of this window is the next window's price to beat.
        assert_eq!(record(&e, NEXT).price_to_beat_ours, Some(100.0));
    }

    #[test]
    fn outcome_down_when_final_below_ptb() {
        let mut e = discovered(None);
        e.handle(twap(START_MS, 100.0), START_MS + 1300);
        e.handle(twap(END_MS, 99.99), END_MS + 100);
        assert_eq!(record(&e, START).outcome_ours, Some(Outcome::Down));
    }

    #[test]
    fn twap_match_true_when_equal_to_gamma_final() {
        let mut e = discovered(None);
        e.handle(twap(START_MS, 100.0), START_MS + 1300);
        e.handle(twap(END_MS, 100.25), END_MS + 100);
        assert_eq!(record(&e, START).twap_match, None, "gamma final unknown");
        let out = e.handle(gamma(START, Some(100.25)), END_MS + 130_000);
        let w = window_outputs(&out, START).pop().unwrap();
        assert_eq!(w.final_price_gamma, Some(100.25));
        assert_eq!(w.twap_match, Some(true));

        let mut e = discovered(Some(100.26));
        e.handle(twap(END_MS, 100.25), END_MS + 100);
        assert_eq!(record(&e, START).twap_match, Some(false));
    }

    #[test]
    fn resolution_latched() {
        let mut e = discovered(None);
        let resolution = |winner, resolved_at| FeedEvent::Resolution {
            condition_id: cond(START),
            winner,
            resolved_at,
        };
        let out = e.handle(
            resolution(Some(Outcome::Down), Some(1793031327)),
            END_MS + 61_000,
        );
        let w = window_outputs(&out, START).pop().unwrap();
        assert_eq!(w.winner, Some(Outcome::Down));
        assert_eq!(w.resolved_at, Some(1793031327));
        assert_eq!(w.resolution_seen_ms, Some(END_MS + 61_000));
        assert_eq!(w.status, WindowStatus::Resolved);

        e.handle(resolution(None, None), END_MS + 66_000);
        let rec = record(&e, START);
        assert_eq!(rec.winner, Some(Outcome::Down));
        assert_eq!(rec.resolved_at, Some(1793031327));
        assert_eq!(rec.resolution_seen_ms, Some(END_MS + 61_000));
    }

    #[test]
    fn market_resolved_ws_sets_winner_by_token() {
        let mut e = discovered(None);
        e.handle(book(&up(START), "0.98", "0.99"), END_MS - 1000);
        e.handle(book(&up(NEXT), "0.50", "0.52"), END_MS - 1000);
        e.tick(END_MS + 1000);
        let out = e.handle(
            FeedEvent::MarketResolved {
                condition_id: cond(START),
                winning_asset_id: up(START),
                winning_outcome: "not-a-label".into(),
                recv_ms: END_MS + 26_000,
            },
            END_MS + 26_050,
        );
        let w = window_outputs(&out, START).pop().unwrap();
        assert_eq!(w.winner, Some(Outcome::Up));
        assert_eq!(w.resolution_seen_ms, Some(END_MS + 26_000));
        assert_eq!(w.resolved_at, None);
        assert_eq!(w.status, WindowStatus::Resolved);
        assert_eq!(
            desired(&out),
            Some(BTreeSet::from([up(NEXT), down(NEXT)])),
            "a resolved window's tokens are no longer wanted"
        );
        assert!(!e.books.contains_key(&up(START)), "its book is dropped");
        assert!(e.books.contains_key(&up(NEXT)));
    }

    #[test]
    fn resolution_requests_start_after_60s_every_5s() {
        let mut e = discovered(None);
        let mut sent = vec![];
        for now in (END_MS..=END_MS + 80_000).step_by(1000) {
            if requests(&e.tick(now)).contains(&fetch_resolution(START)) {
                sent.push((now - END_MS) / 1000);
            }
        }
        assert_eq!(sent, vec![60, 65, 70, 75, 80]);

        e.handle(
            FeedEvent::Resolution {
                condition_id: cond(START),
                winner: Some(Outcome::Up),
                resolved_at: None,
            },
            END_MS + 81_000,
        );
        for now in (END_MS + 81_000..END_MS + 100_000).step_by(1000) {
            assert!(!requests(&e.tick(now)).contains(&fetch_resolution(START)));
        }
    }

    #[test]
    fn gamma_final_polling_times_out() {
        let mut e = discovered(None);
        let (mut gamma_at, mut other_at, mut last_window) = (vec![], vec![], None);
        for now in (END_MS..=END_MS + GIVE_UP_MS + 120_000).step_by(1000) {
            let out = e.tick(now);
            for r in requests(&out) {
                if r == fetch_gamma(START) {
                    gamma_at.push((now - END_MS) / 1000);
                } else if r == fetch_resolution(START) {
                    other_at.push(now - END_MS);
                }
            }
            if let Some(w) = window_outputs(&out, START).pop() {
                last_window = Some(w);
            }
        }
        let expected: Vec<i64> = (120..1800).step_by(30).collect();
        assert_eq!(gamma_at, expected);
        assert!(
            other_at.iter().all(|&t| t < GIVE_UP_MS),
            "resolution polls stop too"
        );
        let w = last_window.expect("status changes were emitted");
        assert_eq!(w.status, WindowStatus::TimedOut);
        assert_eq!(w.updated_ms, END_MS + GIVE_UP_MS);
        assert!(
            !e.windows.contains_key(&WindowId {
                asset: Asset::Btc,
                start: START,
            }),
            "dropped 60 s after timing out"
        );
    }

    #[test]
    fn complete_needs_winner_and_gamma_final_then_is_dropped() {
        let mut e = discovered(Some(100.0));
        e.handle(
            FeedEvent::Resolution {
                condition_id: cond(START),
                winner: Some(Outcome::Up),
                resolved_at: Some(NEXT + 30),
            },
            END_MS + 61_000,
        );
        let out = e.tick(END_MS + 62_000);
        assert_eq!(record(&e, START).status, WindowStatus::Complete);
        assert!(requests(&out).iter().all(|r| match r {
            Request::FetchGamma { slug: s } | Request::FetchResolution { slug: s, .. } =>
                *s != slug(START),
        }));
        // Complete since the resolution arrived at end + 61 s.
        e.tick(END_MS + 61_000 + 59_000);
        assert!(e.windows.keys().any(|id| id.start == START));
        e.tick(END_MS + 61_000 + 60_000);
        assert!(!e.windows.keys().any(|id| id.start == START));
    }

    #[test]
    fn series_rows_only_while_live_and_count_coverage() {
        let mut e = engine();
        let mut live_rows = vec![];
        for i in 0..10 {
            live_rows.extend(rows(&e.tick(START_MS + i * 1000), START));
        }
        assert_eq!(live_rows.len(), 10);
        assert_eq!(live_rows[0].ts, START);
        assert_eq!(live_rows[9].ts, START + 9);
        assert!(live_rows.iter().all(|r| r.twap.is_none()));
        assert_eq!(record(&e, START).coverage_secs, 10);

        for now in [END_MS, END_MS + 1000, END_MS + 2000] {
            let out = e.tick(now);
            assert!(rows(&out, START).is_empty());
            assert_eq!(rows(&out, NEXT).len(), 1, "the next window is live now");
        }
        assert_eq!(record(&e, START).coverage_secs, 10);
    }

    #[test]
    fn series_row_carries_latest_prices() {
        let mut e = discovered(None);
        e.handle(twap(START_MS + 4000, 100.5), START_MS + 4100);
        e.handle(
            FeedEvent::Oracle {
                asset: Asset::Btc,
                kind: OracleKind::Chainlink,
                source: Source::Polybolt,
                ts_ms: START_MS + 4000,
                value: 100.7,
            },
            START_MS + 4100,
        );
        e.handle(
            FeedEvent::CexQuote {
                venue: Venue::Binance,
                asset: Asset::Btc,
                bid: d("100"),
                ask: d("100.2"),
                recv_ms: START_MS + 4200,
            },
            START_MS + 4200,
        );
        e.handle(
            FeedEvent::UsdtUsd {
                price: d("1"),
                recv_ms: START_MS + 4200,
            },
            START_MS + 4200,
        );
        e.handle(book(&up(START), "0.55", "0.57"), START_MS + 4300);
        e.handle(book(&down(START), "0.43", "0.45"), START_MS + 4300);

        let row = rows(&e.tick(START_MS + 5000), START).pop().unwrap();
        assert_eq!(row.twap, Some(100.5));
        assert_eq!(row.chainlink, Some(100.7));
        assert!((row.binance_mid_usd.unwrap() - 100.1).abs() < 1e-9);
        assert_eq!(row.coinbase_mid, None);
        assert_eq!((row.up_bid, row.up_ask), (Some(d("0.55")), Some(d("0.57"))));
        assert_eq!(
            (row.down_bid, row.down_ask),
            (Some(d("0.43")), Some(d("0.45")))
        );

        let live = e.live_windows(START_MS + 5000);
        assert_eq!(live.len(), 1);
        let lw = &live[0];
        assert_eq!(lw.slug, slug(START));
        assert_eq!(lw.secs_left, 295.0);
        assert_eq!(
            lw.price_to_beat,
            Some(99.5),
            "gamma ptb until ours is known"
        );
        assert_eq!(
            (lw.twap, lw.twap_ts_ms),
            (Some(100.5), Some(START_MS + 4000))
        );
        assert_eq!(lw.up_ask, Some(d("0.57")));
    }

    #[test]
    fn book_resync_and_price_change_per_asset() {
        let mut e = discovered(None);
        let first = e.handle(book(&up(START), "0.55", "0.57"), START_MS + 2000);
        assert!(
            !first
                .iter()
                .any(|o| matches!(o, EngineOutput::BookResync { .. }))
        );

        e.handle(
            FeedEvent::PriceChange {
                market: "m".into(),
                ts_ms: 0,
                changes: vec![PriceChangeEntry {
                    asset_id: up(START),
                    price: d("0.56"),
                    size: d("4"),
                    side: Side::Buy,
                    best_bid: Some(d("0.56")),
                    best_ask: Some(d("0.57")),
                }],
            },
            START_MS + 2100,
        );
        assert_eq!(e.books[&up(START)].best_bid(), Some(d("0.56")));

        // A snapshot that disagrees with the delta-built book is a resync.
        let out = e.handle(book(&up(START), "0.55", "0.57"), START_MS + 2200);
        assert!(out.contains(&EngineOutput::BookResync {
            asset_id: up(START)
        }));
        assert_eq!(e.book_resyncs(), 1);

        // An identical snapshot is not.
        let out = e.handle(book(&up(START), "0.55", "0.57"), START_MS + 2300);
        assert!(
            !out.iter()
                .any(|o| matches!(o, EngineOutput::BookResync { .. }))
        );
        assert_eq!(e.book_resyncs(), 1);
    }

    #[test]
    fn feed_reset_clears_books() {
        let mut e = discovered(None);
        e.handle(book(&up(START), "0.55", "0.57"), START_MS + 2000);
        e.handle(book(&down(START), "0.43", "0.45"), START_MS + 2000);

        e.handle(
            FeedEvent::FeedReset {
                source: Source::Binance,
                shard: None,
            },
            START_MS + 2100,
        );
        assert_eq!(e.books.len(), 2, "only a CLOB reset clears books");

        e.handle(
            FeedEvent::FeedReset {
                source: Source::ClobMarket,
                shard: None,
            },
            START_MS + 2200,
        );
        assert!(e.books.is_empty());
        let row = rows(&e.tick(START_MS + 3000), START).pop().unwrap();
        assert_eq!((row.up_bid, row.up_ask), (None, None));

        // The fresh snapshot after a reconnect is a new base, not a resync.
        let out = e.handle(book(&up(START), "0.60", "0.62"), START_MS + 3100);
        assert!(
            !out.iter()
                .any(|o| matches!(o, EngineOutput::BookResync { .. }))
        );
        assert_eq!(e.book_resyncs(), 0);
    }

    fn eth_gamma(start: i64) -> FeedEvent {
        let FeedEvent::GammaFetched {
            window: Some(mut w),
            ..
        } = gamma(start, None)
        else {
            unreachable!()
        };
        w.slug = format!("eth-updown-5m-{start}");
        w.up_token = format!("eth-up-{start}");
        w.down_token = format!("eth-down-{start}");
        w.condition_id = format!("eth-cond-{start}");
        FeedEvent::GammaFetched {
            slug: w.slug.clone(),
            window: Some(w),
            recv_ms: 0,
        }
    }

    fn two_asset_engine() -> Engine {
        let mut e = Engine::new(
            EngineConfig {
                assets: vec![Asset::Btc, Asset::Eth],
                session_id: "session-1".into(),
                shadow: None,
            },
            START_MS,
        );
        e.tick(START_MS + 1000);
        e.handle(gamma(START, None), START_MS + 1100);
        e.handle(eth_gamma(START), START_MS + 1100);
        e
    }

    #[test]
    fn desired_tokens_are_split_per_asset() {
        let mut e = Engine::new(
            EngineConfig {
                assets: vec![Asset::Btc, Asset::Eth],
                session_id: "session-1".into(),
                shadow: None,
            },
            START_MS,
        );
        e.tick(START_MS + 1000);
        let out = e.handle(eth_gamma(START), START_MS + 1100);
        let all = out
            .iter()
            .rev()
            .find_map(|o| match o {
                EngineOutput::DesiredTokens(t) => Some(t.clone()),
                _ => None,
            })
            .expect("desired tokens");
        assert_eq!(
            all,
            DesiredTokens::from([
                (Asset::Btc, BTreeSet::new()),
                (
                    Asset::Eth,
                    BTreeSet::from([format!("eth-up-{START}"), format!("eth-down-{START}")])
                ),
            ])
        );
    }

    #[test]
    fn clob_shard_reset_clears_only_that_assets_books() {
        let mut e = two_asset_engine();
        e.handle(book(&up(START), "0.55", "0.57"), START_MS + 2000);
        e.handle(
            book(&format!("eth-up-{START}"), "0.40", "0.42"),
            START_MS + 2000,
        );
        e.handle(
            FeedEvent::FeedReset {
                source: Source::ClobMarket,
                shard: Some(Asset::Eth),
            },
            START_MS + 2100,
        );
        assert!(e.books.contains_key(&up(START)), "btc book kept");
        assert!(!e.books.contains_key(&format!("eth-up-{START}")));
    }

    #[test]
    fn adopted_window_keeps_latched_fields_and_completes() {
        // Session A recorded the price to beat and the final TWAP, then stopped before Gamma's
        // final price arrived. Session B adopts the row long after the normal give-up.
        let mut stored = record(&discovered(None), START);
        stored.price_to_beat_ours = Some(100.0);
        stored.twap_source = Some(Source::Polybolt);
        stored.final_twap_ours = Some(100.25);
        stored.final_twap_source = Some(Source::Polybolt);
        stored.outcome_ours = Some(Outcome::Up);
        stored.winner = Some(Outcome::Up);
        stored.status = WindowStatus::TimedOut;
        stored.coverage_secs = 120;
        let later = END_MS + 2 * GIVE_UP_MS;

        let mut e = engine();
        e.adopt(stored.clone(), later);
        let out = e.tick(later);
        assert!(
            requests(&out).contains(&fetch_gamma(START)),
            "the adopted window is polled at once"
        );
        let rec = record(&e, START);
        assert_eq!(rec.price_to_beat_ours, Some(100.0), "latched, not reset");
        assert_eq!(rec.twap_source, Some(Source::Polybolt));
        assert_eq!(
            rec.status,
            WindowStatus::Resolved,
            "in its grace, not timed out"
        );
        assert_eq!(rec.coverage_secs, 120);

        let out = e.handle(gamma(START, Some(100.25)), later + 500);
        let w = window_outputs(&out, START).pop().expect("completed");
        assert_eq!(w.twap_match, Some(true));
        assert_eq!(w.status, WindowStatus::Complete);
        assert_eq!(w.session_id, stored.session_id);

        // Adopting a window that is already tracked changes nothing.
        let mut other = stored.clone();
        other.price_to_beat_ours = Some(1.0);
        e.adopt(other, later + 600);
        assert_eq!(record(&e, START).price_to_beat_ours, Some(100.0));
    }

    #[test]
    fn wake_after_long_sleep_polls_before_timing_out() {
        let mut e = discovered(None);
        e.tick(END_MS - 1000);
        // Asleep for an hour: the first tick after the wake must poll, not time out.
        let wake = END_MS + 60 * 60_000;
        let out = e.tick(wake);
        let reqs = requests(&out);
        assert!(reqs.contains(&fetch_gamma(START)), "{reqs:?}");
        assert!(reqs.contains(&fetch_resolution(START)), "{reqs:?}");
        assert_ne!(record(&e, START).status, WindowStatus::TimedOut);

        let out = e.handle(
            FeedEvent::Resolution {
                condition_id: cond(START),
                winner: Some(Outcome::Down),
                resolved_at: None,
            },
            wake + 1_000,
        );
        assert_eq!(
            window_outputs(&out, START).pop().unwrap().winner,
            Some(Outcome::Down)
        );
        // Without a Gamma final it times out once the grace is over.
        for now in (wake + 2_000..=wake + RESUME_GRACE_MS).step_by(1000) {
            e.tick(now);
        }
        assert_eq!(record(&e, START).status, WindowStatus::TimedOut);
    }

    #[test]
    fn records_lists_every_tracked_window() {
        let e = discovered(None);
        let slugs: Vec<String> = e.records().into_iter().map(|r| r.slug).collect();
        assert_eq!(slugs, vec![slug(START), slug(NEXT)]);
    }

    #[test]
    fn control_is_reachable_and_starts_paused() {
        let mut e = engine();
        assert_eq!(
            e.control().state().stop_kind,
            Some(raki_core::StopKind::Startup)
        );
    }

    #[test]
    fn shadow_predicts_at_checkpoints_and_fills_v0_against_the_later_book() {
        use raki_core::{SafetyChecks, ShadowAction, ShadowFillStatus};
        use raki_strategy::model::ModelParams;

        use crate::shadow::{CHECKPOINTS, ShadowConfig};

        let t0 = (START - 120) * 1000;
        let mut e = Engine::new(
            EngineConfig {
                assets: vec![Asset::Btc],
                session_id: "session-1".into(),
                shadow: Some(ShadowConfig {
                    model: ModelParams {
                        vol_min_points: 10,
                        ..ModelParams::default()
                    },
                    ..ShadowConfig::default()
                }),
            },
            t0,
        );
        let ok = SafetyChecks {
            feeds_fresh: true,
            clock_ok: true,
            geoblock_ok: true,
            recorder_ok: true,
            store_ok: true,
            all_ok: true,
            reasons: vec![],
        };
        e.control().on_safety(&ok, t0);
        // A failing check pauses the bot until a manual resume; once the checks pass again the
        // shadow still evaluates, marked as made while paused.
        let failing = SafetyChecks {
            all_ok: false,
            reasons: vec!["feed stale: binance".into()],
            ..ok.clone()
        };
        e.control().on_safety(&failing, t0);
        e.control().on_safety(&ok, t0);
        assert_eq!(e.control().state().mode, TradingMode::Paused);
        // Spot at 100 ± 0.05 (σ = 0.1 per second) against a price to beat of 99.0: Up firms up
        // as the window runs out.
        let spot = |s: i64| FeedEvent::Oracle {
            asset: Asset::Btc,
            kind: OracleKind::Chainlink,
            source: Source::Polybolt,
            ts_ms: s * 1000,
            value: 100.0 + if s % 2 == 0 { 0.05 } else { -0.05 },
        };
        let mut outs = vec![];
        for s in START - 120..NEXT {
            let now = s * 1000 + 100;
            outs.extend(e.handle(spot(s), now));
            if s == START - 119 {
                outs.extend(e.handle(gamma(START, None), now));
            }
            if s == START {
                outs.extend(e.handle(twap(START_MS, 99.0), now));
                outs.extend(e.handle(book(&up(START), "0.95", "0.96"), now));
                outs.extend(e.handle(book(&down(START), "0.03", "0.05"), now));
            }
            outs.extend(e.tick(now));
            if NEXT - s == 45 {
                // The decision (limit 0.96) was just taken; before it is due (250 ms) the ask
                // moves to 0.97, so the fill at the next event finds nothing at the limit.
                outs.extend(e.handle(book(&up(START), "0.95", "0.97"), now + 200));
                outs.extend(e.handle(book(&up(START), "0.95", "0.97"), now + 300));
            }
        }

        let preds: Vec<_> = outs
            .iter()
            .filter_map(|o| match o {
                EngineOutput::Prediction(p) if p.slug == slug(START) => Some(p),
                _ => None,
            })
            .collect();
        let checkpoints: Vec<i64> = preds.iter().map(|p| p.checkpoint_s).collect();
        assert_eq!(checkpoints, CHECKPOINTS);
        assert!(preds.iter().all(|p| p.ptb == 99.0), "our price to beat wins over Gamma's");
        assert!(
            preds.windows(2).all(|w| w[1].sd < w[0].sd),
            "the forecast narrows as the window runs out"
        );
        assert!(preds[0].p_up < 0.9 && preds.iter().skip(5).all(|p| p.p_up > 0.99));

        let evals: Vec<_> = outs
            .iter()
            .filter_map(|o| match o {
                EngineOutput::ShadowEval(ev) if ev.slug == slug(START) => Some(ev),
                _ => None,
            })
            .collect();
        assert_eq!(evals.len(), 2, "{evals:?}");
        // The previous window (no price to beat) used the first 16 numbers, skipping.
        assert_eq!((evals[0].seq, evals[0].secs_left), (16, 45));
        assert_eq!(evals[0].action, ShadowAction::Enter);
        assert!(evals.iter().all(|ev| !ev.control_running));
        assert_eq!(evals[0].best_ask, Some(d("0.96")));
        assert_eq!((evals[1].seq, evals[1].secs_left), (17, 44));
        assert_eq!(evals[1].best_ask, Some(d("0.97")));

        let fills: Vec<_> = outs
            .iter()
            .filter_map(|o| match o {
                EngineOutput::ShadowFill(f) => Some(f),
                _ => None,
            })
            .collect();
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].status, ShadowFillStatus::NoFill);
        assert_eq!(fills[0].filled_ms, (NEXT - 45) * 1000 + 400);
        assert_eq!(fills[1].status, ShadowFillStatus::Filled);
        assert_eq!((fills[1].seq, fills[1].limit_price), (17, d("0.97")));
        assert_eq!(fills[1].cost, d("5"));
        assert_eq!(fills[1].side, Outcome::Up);

        let live = e.live_windows(END_MS - 900);
        let shadow = live[0].shadow.as_ref().unwrap();
        assert_eq!(
            shadow.fill.as_ref().map(|f| f.status),
            Some(ShadowFillStatus::Filled)
        );
        assert!(!shadow.pending);
        assert!(live[0].prediction.as_ref().unwrap().p_up > 0.99);
    }

}

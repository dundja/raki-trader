//! Shadow predictions and strategy v0 shadow decisions inside the engine
//! (docs/specs/2026-10-31-shadow-predictions.md §3–4). Pure like the rest of the engine:
//! everything follows from events and `now_ms`, so a replay reproduces it.

use std::collections::{BTreeSet, HashMap};

use raki_core::{
    Asset, LivePrediction, LiveShadow, OracleKind, Outcome, PredictionRow, ShadowAction,
    ShadowEvalRow, ShadowFillRow, ShadowFillStatus, WindowStatus,
};
use raki_strategy::model::{MODEL_ID, ModelParams, Prediction, predict};
use raki_strategy::v0::{
    Decision, EvalInput, Evaluation, Fill, FillStatus, MarketConfig, STRATEGY_ID, SideBook,
    V0Params, V0Window,
};
use rust_decimal::Decimal;

use crate::book::BookState;
use crate::cex::CexState;
use crate::engine::EngineOutput;
use crate::oracle::OracleState;
use crate::series::WindowPrices;
use crate::windows::WindowState;

pub use raki_core::PREDICTION_CHECKPOINTS as CHECKPOINTS;
/// A checkpoint is stored at the first tick at or below it, but not when tracking reached the
/// window this much later (started mid-window, or woke from sleep): a late row is not that
/// checkpoint.
const CHECKPOINT_SLACK_S: i64 = 3;

#[derive(Clone, Debug)]
pub struct ShadowConfig {
    pub model: ModelParams,
    pub v0: V0Params,
    /// Assets v0 evaluates (BTC only, CLAUDE.md §3). Predictions cover every asset.
    pub v0_assets: Vec<Asset>,
}

impl Default for ShadowConfig {
    fn default() -> ShadowConfig {
        ShadowConfig {
            model: ModelParams::default(),
            v0: V0Params::default(),
            v0_assets: vec![Asset::Btc],
        }
    }
}

impl ShadowConfig {
    pub fn params_json(&self) -> serde_json::Value {
        serde_json::json!({
            "model_id": MODEL_ID,
            "model": self.model,
            "strategy_id": STRATEGY_ID,
            "v0": self.v0,
            "v0_assets": self.v0_assets,
        })
    }

    pub fn params_hash(&self) -> String {
        raki_strategy::params_hash(&self.params_json())
    }
}

/// A window's shadow state.
#[derive(Clone, Debug, Default)]
pub struct WindowShadow {
    checkpoints_done: BTreeSet<i64>,
    pub latest: Option<Prediction>,
    pub v0: V0Window,
    pub last_eval: Option<ShadowEvalRow>,
    pub fill: Option<ShadowFillRow>,
}

impl WindowShadow {
    pub fn live_prediction(&self) -> Option<LivePrediction> {
        self.latest.as_ref().map(|p| LivePrediction {
            p_up: p.p_up,
            mu: p.mu,
            sd: p.sd,
            spot_ts_ms: p.spot_ts * 1000,
        })
    }

    pub fn live_shadow(&self) -> LiveShadow {
        LiveShadow {
            last_eval: self.last_eval.clone(),
            fill: self.fill.clone(),
            pending: self.v0.pending().is_some(),
        }
    }
}

/// What the shadow reads from the rest of the engine.
pub struct Ctx<'a> {
    pub cfg: &'a ShadowConfig,
    pub params_hash: &'a str,
    pub session_id: &'a str,
    pub oracle: &'a OracleState,
    pub cex: &'a CexState,
    pub books: &'a HashMap<String, BookState>,
    /// Control is `running`.
    pub running: bool,
}

/// One tick for one window: refresh its forecast, store due checkpoints, and (v0 assets, entry
/// window) evaluate v0. Only live windows.
pub fn on_tick(
    ctx: &Ctx<'_>,
    w: &mut WindowState,
    now_ms: i64,
    seq: &mut u64,
    out: &mut Vec<EngineOutput>,
) {
    if w.rec.status != WindowStatus::Live {
        return;
    }
    let secs_left = w.id.end() - now_ms.div_euclid(1000);
    w.shadow.latest = forecast(ctx, w);

    let due: Vec<i64> = CHECKPOINTS
        .iter()
        .copied()
        .filter(|&c| secs_left <= c && !w.shadow.checkpoints_done.contains(&c))
        .collect();
    for c in due {
        w.shadow.checkpoints_done.insert(c);
        if secs_left > c - CHECKPOINT_SLACK_S
            && let Some(row) = prediction_row(ctx, w, c, now_ms)
        {
            out.push(EngineOutput::Prediction(row));
        }
    }

    if ctx.cfg.v0_assets.contains(&w.id.asset) && ctx.cfg.v0.in_entry_window(secs_left) {
        let input = EvalInput {
            running: ctx.running,
            p_up: w.shadow.latest.as_ref().map(|p| p.p_up),
            config: market_config(w),
            up: side_book(ctx.books, w.rec.up_token.as_deref()),
            down: side_book(ctx.books, w.rec.down_token.as_deref()),
        };
        if let Some(ev) = w.shadow.v0.evaluate(&ctx.cfg.v0, &input, *seq, now_ms) {
            let row = eval_row(ctx, w, *seq, now_ms, secs_left, &ev);
            *seq += 1;
            w.shadow.last_eval = Some(row.clone());
            out.push(EngineOutput::ShadowEval(row));
        }
    }
    fill_due(ctx.session_id, ctx.books, w, now_ms, out);
}

/// Fills the window's pending v0 intent once it is due, against its side's asks as they are now.
/// Called before every book event is applied and on every tick, so the fill sees the book as it
/// was at the due time (to within one event).
pub fn fill_due(
    session_id: &str,
    books: &HashMap<String, BookState>,
    w: &mut WindowState,
    now_ms: i64,
    out: &mut Vec<EngineOutput>,
) {
    let Some(intent) = w.shadow.v0.pending() else {
        return;
    };
    if intent.due_ms > now_ms {
        return;
    }
    let token = match intent.side {
        Outcome::Up => w.rec.up_token.as_deref(),
        Outcome::Down => w.rec.down_token.as_deref(),
    };
    let asks = token
        .and_then(|t| books.get(t))
        .map(BookState::asks)
        .unwrap_or_default();
    if let Some(fill) = w.shadow.v0.fill_due(now_ms, &asks) {
        let row = fill_row(session_id, &w.rec.slug, &fill);
        w.shadow.fill = Some(row.clone());
        out.push(EngineOutput::ShadowFill(row));
    }
}

fn forecast(ctx: &Ctx<'_>, w: &WindowState) -> Option<Prediction> {
    let ptb = w.rec.price_to_beat_ours.or(w.rec.price_to_beat_gamma)?;
    let lookback = w.gamma.as_ref()?.twap_lookback_s?;
    let spot = ctx.oracle.seconds(w.id.asset, OracleKind::Chainlink);
    predict(&ctx.cfg.model, ptb, w.id.end(), lookback, &spot)
}

fn market_config(w: &WindowState) -> Result<MarketConfig, String> {
    let g = w.gamma.as_ref().ok_or("no gamma config")?;
    MarketConfig::from_gamma(g.fee_rate, g.fee_exponent, g.order_min_size, g.tick_size)
}

fn side_book(books: &HashMap<String, BookState>, token: Option<&str>) -> Option<SideBook> {
    let book = books.get(token?)?;
    Some(SideBook {
        best_bid: book.best_bid(),
        asks: book.asks(),
    })
}

fn prediction_row(
    ctx: &Ctx<'_>,
    w: &WindowState,
    checkpoint_s: i64,
    now_ms: i64,
) -> Option<PredictionRow> {
    let p = w.shadow.latest.as_ref()?;
    let ptb = w.rec.price_to_beat_ours.or(w.rec.price_to_beat_gamma)?;
    let prices = WindowPrices::collect(
        w.id.asset,
        w.rec.up_token.as_deref(),
        w.rec.down_token.as_deref(),
        ctx.oracle,
        ctx.cex,
        ctx.books,
    );
    Some(PredictionRow {
        slug: w.rec.slug.clone(),
        asset: w.id.asset,
        checkpoint_s,
        ts_ms: now_ms,
        p_up: p.p_up,
        mu: p.mu,
        sd: p.sd,
        sigma: p.sigma,
        ptb,
        spot: p.spot,
        twap: prices.twap.map(|(_, v)| v),
        binance_mid_usd: prices.binance_mid_usd,
        coinbase_mid: prices.coinbase_mid,
        up_bid: prices.up_bid,
        up_ask: prices.up_ask,
        down_bid: prices.down_bid,
        down_ask: prices.down_ask,
        model_id: MODEL_ID.to_owned(),
        session_id: ctx.session_id.to_owned(),
    })
}

fn eval_row(
    ctx: &Ctx<'_>,
    w: &WindowState,
    seq: u64,
    now_ms: i64,
    secs_left: i64,
    ev: &Evaluation,
) -> ShadowEvalRow {
    let walk = ev.walk.as_ref();
    let (action, reason) = match &ev.decision {
        Decision::Enter { .. } => (ShadowAction::Enter, None),
        Decision::Skip(r) => (ShadowAction::Skip, Some(r.as_str().to_owned())),
    };
    ShadowEvalRow {
        session_id: ctx.session_id.to_owned(),
        seq: seq as i64,
        strategy_id: STRATEGY_ID.to_owned(),
        params_hash: ctx.params_hash.to_owned(),
        slug: w.rec.slug.clone(),
        ts_ms: now_ms,
        secs_left,
        side: ev.side,
        model_prob: ev.model_prob,
        market_prob: ev.market_prob,
        best_ask: ev.best_ask,
        walk_shares: walk.map(|w| w.shares),
        walk_avg_price: walk.and_then(|w| w.avg_price()),
        walk_fee: walk.map(|w| w.fee),
        edge: ev.edge,
        action,
        reason,
    }
}

fn fill_row(session_id: &str, slug: &str, f: &Fill) -> ShadowFillRow {
    ShadowFillRow {
        session_id: session_id.to_owned(),
        seq: f.intent.seq as i64,
        slug: slug.to_owned(),
        side: f.intent.side,
        decided_ms: f.intent.decided_ms,
        due_ms: f.intent.due_ms,
        filled_ms: f.filled_ms,
        limit_price: f.intent.limit,
        shares: f.walk.shares,
        cost: f.walk.cost,
        fee: f.walk.fee,
        status: match f.status {
            FillStatus::Filled => ShadowFillStatus::Filled,
            FillStatus::Partial => ShadowFillStatus::Partial,
            FillStatus::NoFill => ShadowFillStatus::NoFill,
        },
        winner: None,
        pnl: None::<Decimal>,
    }
}

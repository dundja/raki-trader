//! Shadow predictions and strategy v0 shadow decisions (docs/specs/2026-10-31-shadow-predictions.md).
//! Every insert is idempotent (`on conflict do nothing`), so the writer can retry safely.

use anyhow::{Context, Result};
use raki_core::{
    Asset, Calibration, CalibrationBucket, Outcome, PredictionRow, ReasonCount, ShadowAction,
    ShadowEvalRow, ShadowFillRow, ShadowFillStatus, ShadowSummary,
};
use rust_decimal::Decimal;
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use super::{Store, enum_from_str, enum_to_str, opt_enum_from_str, opt_enum_to_str};

/// Calibration buckets of the favourite's predicted probability (`max(p_up, 1 − p_up)`).
const BUCKETS: [(f64, f64, &str); 6] = [
    (0.5, 0.8, "0.50–0.80"),
    (0.8, 0.92, "0.80–0.92"),
    (0.92, 0.95, "0.92–0.95"),
    (0.95, 0.97, "0.95–0.97"),
    (0.97, 0.99, "0.97–0.99"),
    (0.99, 1.0, "≥ 0.99"),
];

/// PnL of a fill once its window has a winner: the shares pay $1 if the side won.
const PNL_SQL: &str = "case when w.winner = f.side then f.shares - f.cost - f.fee \
                       else -(f.cost + f.fee) end";

impl Store {
    /// Records a parameter set the first time it is used.
    pub async fn upsert_strategy_params(
        &self,
        params_hash: &str,
        strategy_id: &str,
        params: &serde_json::Value,
        now_ms: i64,
    ) -> Result<()> {
        sqlx::query(
            "insert into strategy_params (params_hash, strategy_id, params, first_seen_ms) \
             values ($1, $2, $3::jsonb, $4) on conflict do nothing",
        )
        .bind(params_hash)
        .bind(strategy_id)
        .bind(params.to_string())
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .context("insert strategy params")?;
        Ok(())
    }

    pub async fn insert_prediction(&self, p: &PredictionRow) -> Result<()> {
        sqlx::query(
            "insert into predictions (slug, asset, checkpoint_s, ts_ms, p_up, mu, sd, sigma, ptb, \
             spot, twap, binance_mid_usd, coinbase_mid, up_bid, up_ask, down_bid, down_ask, \
             model_id, session_id) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
             $13, $14, $15, $16, $17, $18, $19) on conflict do nothing",
        )
        .bind(&p.slug)
        .bind(p.asset.as_str())
        .bind(p.checkpoint_s as i32)
        .bind(p.ts_ms)
        .bind(p.p_up)
        .bind(p.mu)
        .bind(p.sd)
        .bind(p.sigma)
        .bind(p.ptb)
        .bind(p.spot)
        .bind(p.twap)
        .bind(p.binance_mid_usd)
        .bind(p.coinbase_mid)
        .bind(p.up_bid)
        .bind(p.up_ask)
        .bind(p.down_bid)
        .bind(p.down_ask)
        .bind(&p.model_id)
        .bind(parse_uuid(&p.session_id)?)
        .execute(&self.pool)
        .await
        .context("insert prediction")?;
        Ok(())
    }

    pub async fn insert_shadow_eval(&self, e: &ShadowEvalRow) -> Result<()> {
        sqlx::query(
            "insert into shadow_evals (session_id, seq, strategy_id, params_hash, slug, ts_ms, \
             secs_left, side, model_prob, market_prob, best_ask, walk_shares, walk_avg_price, \
             walk_fee, edge, action, reason, control_running) values ($1, $2, $3, $4, $5, $6, $7, \
             $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18) on conflict do nothing",
        )
        .bind(parse_uuid(&e.session_id)?)
        .bind(e.seq)
        .bind(&e.strategy_id)
        .bind(&e.params_hash)
        .bind(&e.slug)
        .bind(e.ts_ms)
        .bind(e.secs_left as i32)
        .bind(opt_enum_to_str(&e.side)?)
        .bind(e.model_prob)
        .bind(e.market_prob)
        .bind(e.best_ask)
        .bind(e.walk_shares)
        .bind(e.walk_avg_price)
        .bind(e.walk_fee)
        .bind(e.edge)
        .bind(enum_to_str(&e.action)?)
        .bind(&e.reason)
        .bind(e.control_running)
        .execute(&self.pool)
        .await
        .context("insert shadow eval")?;
        Ok(())
    }

    pub async fn insert_shadow_fill(&self, f: &ShadowFillRow) -> Result<()> {
        sqlx::query(
            "insert into shadow_fills (session_id, seq, slug, side, decided_ms, due_ms, \
             filled_ms, limit_price, shares, cost, fee, status) values ($1, $2, $3, $4, $5, $6, \
             $7, $8, $9, $10, $11, $12) on conflict do nothing",
        )
        .bind(parse_uuid(&f.session_id)?)
        .bind(f.seq)
        .bind(&f.slug)
        .bind(enum_to_str(&f.side)?)
        .bind(f.decided_ms)
        .bind(f.due_ms)
        .bind(f.filled_ms)
        .bind(f.limit_price)
        .bind(f.shares)
        .bind(f.cost)
        .bind(f.fee)
        .bind(enum_to_str(&f.status)?)
        .execute(&self.pool)
        .await
        .context("insert shadow fill")?;
        Ok(())
    }

    /// A window's predictions, earliest checkpoint (most seconds left) first.
    pub async fn window_predictions(&self, slug: &str) -> Result<Vec<PredictionRow>> {
        sqlx::query("select * from predictions where slug = $1 order by checkpoint_s desc")
            .bind(slug)
            .fetch_all(&self.pool)
            .await
            .context("window predictions")?
            .iter()
            .map(prediction_from_row)
            .collect()
    }

    /// A window's v0 evaluations in time order.
    pub async fn window_shadow_evals(&self, slug: &str) -> Result<Vec<ShadowEvalRow>> {
        sqlx::query("select * from shadow_evals where slug = $1 order by ts_ms, seq")
            .bind(slug)
            .fetch_all(&self.pool)
            .await
            .context("window shadow evals")?
            .iter()
            .map(eval_from_row)
            .collect()
    }

    /// Simulated fills, newest first, with the window's winner and the PnL once resolved.
    pub async fn shadow_fills(&self, slug: Option<&str>, limit: i64) -> Result<Vec<ShadowFillRow>> {
        let sql = format!(
            "select f.*, w.winner, \
             case when w.winner is not null and f.status <> 'no_fill' then {PNL_SQL} end as pnl \
             from shadow_fills f left join windows w on w.slug = f.slug \
             where ($1::text is null or f.slug = $1) order by f.filled_ms desc limit $2"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(slug)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .context("shadow fills")?
            .iter()
            .map(fill_from_row)
            .collect()
    }

    pub async fn shadow_summary(&self) -> Result<ShadowSummary> {
        let evals = sqlx::query(
            "select count(*) as evaluations, \
             count(*) filter (where not control_running) as while_paused, \
             count(distinct slug) as windows, \
             count(distinct slug) filter (where action = 'enter') as windows_entered, \
             count(*) filter (where action = 'enter') as entries from shadow_evals",
        )
        .fetch_one(&self.pool)
        .await
        .context("shadow eval counts")?;
        // `r`: a fill (not a no-fill) whose window has a winner.
        let r = "w.winner is not null and f.status <> 'no_fill'";
        let sql = format!(
            "select count(*) filter (where f.status = 'filled') as filled, \
             count(*) filter (where f.status = 'partial') as partial, \
             count(*) filter (where f.status = 'no_fill') as no_fill, \
             count(*) filter (where {r}) as resolved, \
             count(*) filter (where {r} and w.winner = f.side) as wins, \
             count(*) filter (where {r} and w.winner <> f.side) as losses, \
             coalesce(sum(f.cost + f.fee) filter (where {r}), 0) as staked, \
             coalesce(sum({PNL_SQL}) filter (where {r}), 0) as pnl \
             from shadow_fills f left join windows w on w.slug = f.slug"
        );
        let fills = sqlx::query(sqlx::AssertSqlSafe(sql))
            .fetch_one(&self.pool)
            .await
            .context("shadow fill counts")?;
        let edge: Option<f64> = sqlx::query_scalar(
            "select avg(e.edge)::float8 from shadow_evals e \
             join shadow_fills f on f.session_id = e.session_id and f.seq = e.seq \
             where f.status <> 'no_fill'",
        )
        .fetch_one(&self.pool)
        .await
        .context("shadow mean edge")?;
        let skip_reasons = sqlx::query(
            "select reason, count(*) as n from shadow_evals where action = 'skip' \
             group by reason order by n desc, reason",
        )
        .fetch_all(&self.pool)
        .await
        .context("shadow skip reasons")?
        .iter()
        .map(|r| {
            Ok(ReasonCount {
                reason: r.try_get::<Option<String>, _>("reason")?.unwrap_or_default(),
                count: r.try_get("n")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
        let params_hashes = sqlx::query_scalar(
            "select params_hash from shadow_evals group by params_hash order by min(ts_ms)",
        )
        .fetch_all(&self.pool)
        .await
        .context("shadow params hashes")?;
        Ok(ShadowSummary {
            evaluations: evals.try_get("evaluations")?,
            evaluations_while_paused: evals.try_get("while_paused")?,
            windows_evaluated: evals.try_get("windows")?,
            windows_entered: evals.try_get("windows_entered")?,
            entries: evals.try_get("entries")?,
            filled: fills.try_get("filled")?,
            partial: fills.try_get("partial")?,
            no_fill: fills.try_get("no_fill")?,
            resolved: fills.try_get("resolved")?,
            wins: fills.try_get("wins")?,
            losses: fills.try_get("losses")?,
            staked: fills.try_get("staked")?,
            pnl: fills.try_get("pnl")?,
            mean_predicted_edge: edge,
            skip_reasons,
            params_hashes,
        })
    }

    /// Predicted vs realized at `checkpoint_s` over windows with a winner.
    pub async fn calibration(&self, checkpoint_s: i64, asset: Option<Asset>) -> Result<Calibration> {
        let rows = sqlx::query(
            "select p.p_up, w.winner from predictions p join windows w on w.slug = p.slug \
             where p.checkpoint_s = $1 and w.winner is not null \
             and ($2::text is null or p.asset = $2)",
        )
        .bind(checkpoint_s as i32)
        .bind(asset.map(|a| a.as_str()))
        .fetch_all(&self.pool)
        .await
        .context("calibration rows")?;
        let points = rows
            .iter()
            .map(|r| {
                let winner = enum_from_str::<Outcome>(&r.try_get::<String, _>("winner")?)?;
                Ok((r.try_get::<f64, _>("p_up")?, winner == Outcome::Up))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(calibrate(checkpoint_s, asset, &points))
    }
}

/// Buckets `(p_up, up_won)` by the favourite's probability; Brier score over `p_up`.
pub fn calibrate(checkpoint_s: i64, asset: Option<Asset>, points: &[(f64, bool)]) -> Calibration {
    let n = points.len();
    let brier = (n > 0).then(|| {
        points
            .iter()
            .map(|&(p, up)| (p - if up { 1.0 } else { 0.0 }).powi(2))
            .sum::<f64>()
            / n as f64
    });
    let buckets = BUCKETS
        .iter()
        .map(|&(lo, hi, label)| {
            let inside: Vec<(f64, bool)> = points
                .iter()
                .map(|&(p, up)| {
                    // The favourite is Up at p ≥ 0.5 (a 0.5 forecast favours Up, like a tie).
                    if p >= 0.5 { (p, up) } else { (1.0 - p, !up) }
                })
                .filter(|&(q, _)| q >= lo && (q < hi || hi >= 1.0))
                .collect();
            let k = inside.len();
            let wins = inside.iter().filter(|(_, won)| *won).count();
            let (ci_lo, ci_hi) = wilson(wins, k).unzip();
            CalibrationBucket {
                label: label.to_owned(),
                lo,
                hi,
                n: k as i64,
                mean_predicted: (k > 0).then(|| inside.iter().map(|(q, _)| q).sum::<f64>() / k as f64),
                realized: (k > 0).then(|| wins as f64 / k as f64),
                ci_lo,
                ci_hi,
            }
        })
        .collect();
    Calibration {
        checkpoint_s,
        asset,
        n: n as i64,
        brier,
        buckets,
    }
}

/// Wilson score 95% interval for `wins` successes out of `n`.
fn wilson(wins: usize, n: usize) -> Option<(f64, f64)> {
    if n == 0 {
        return None;
    }
    let z = 1.959_963_984_540_054_f64;
    let (n, p) = (n as f64, wins as f64 / n as f64);
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denom;
    Some(((center - half).max(0.0), (center + half).min(1.0)))
}

fn parse_uuid(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).with_context(|| format!("bad session id {s:?}"))
}

fn prediction_from_row(r: &PgRow) -> Result<PredictionRow> {
    Ok(PredictionRow {
        slug: r.try_get("slug")?,
        asset: enum_from_str::<Asset>(&r.try_get::<String, _>("asset")?)?,
        checkpoint_s: i64::from(r.try_get::<i32, _>("checkpoint_s")?),
        ts_ms: r.try_get("ts_ms")?,
        p_up: r.try_get("p_up")?,
        mu: r.try_get("mu")?,
        sd: r.try_get("sd")?,
        sigma: r.try_get("sigma")?,
        ptb: r.try_get("ptb")?,
        spot: r.try_get("spot")?,
        twap: r.try_get("twap")?,
        binance_mid_usd: r.try_get("binance_mid_usd")?,
        coinbase_mid: r.try_get("coinbase_mid")?,
        up_bid: r.try_get::<Option<Decimal>, _>("up_bid")?,
        up_ask: r.try_get::<Option<Decimal>, _>("up_ask")?,
        down_bid: r.try_get::<Option<Decimal>, _>("down_bid")?,
        down_ask: r.try_get::<Option<Decimal>, _>("down_ask")?,
        model_id: r.try_get("model_id")?,
        session_id: r.try_get::<Uuid, _>("session_id")?.to_string(),
    })
}

fn eval_from_row(r: &PgRow) -> Result<ShadowEvalRow> {
    Ok(ShadowEvalRow {
        session_id: r.try_get::<Uuid, _>("session_id")?.to_string(),
        seq: r.try_get("seq")?,
        strategy_id: r.try_get("strategy_id")?,
        params_hash: r.try_get("params_hash")?,
        slug: r.try_get("slug")?,
        ts_ms: r.try_get("ts_ms")?,
        secs_left: i64::from(r.try_get::<i32, _>("secs_left")?),
        side: opt_enum_from_str::<Outcome>(r.try_get("side")?)?,
        model_prob: r.try_get("model_prob")?,
        market_prob: r.try_get::<Option<Decimal>, _>("market_prob")?,
        best_ask: r.try_get::<Option<Decimal>, _>("best_ask")?,
        walk_shares: r.try_get::<Option<Decimal>, _>("walk_shares")?,
        walk_avg_price: r.try_get::<Option<Decimal>, _>("walk_avg_price")?,
        walk_fee: r.try_get::<Option<Decimal>, _>("walk_fee")?,
        edge: r.try_get::<Option<Decimal>, _>("edge")?,
        action: enum_from_str::<ShadowAction>(&r.try_get::<String, _>("action")?)?,
        reason: r.try_get("reason")?,
        control_running: r.try_get("control_running")?,
    })
}

fn fill_from_row(r: &PgRow) -> Result<ShadowFillRow> {
    Ok(ShadowFillRow {
        session_id: r.try_get::<Uuid, _>("session_id")?.to_string(),
        seq: r.try_get("seq")?,
        slug: r.try_get("slug")?,
        side: enum_from_str::<Outcome>(&r.try_get::<String, _>("side")?)?,
        decided_ms: r.try_get("decided_ms")?,
        due_ms: r.try_get("due_ms")?,
        filled_ms: r.try_get("filled_ms")?,
        limit_price: r.try_get("limit_price")?,
        shares: r.try_get("shares")?,
        cost: r.try_get("cost")?,
        fee: r.try_get("fee")?,
        status: enum_from_str::<ShadowFillStatus>(&r.try_get::<String, _>("status")?)?,
        winner: opt_enum_from_str::<Outcome>(r.try_get("winner")?)?,
        pnl: r.try_get::<Option<Decimal>, _>("pnl")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_buckets_by_favourite_probability() {
        let points = [
            (0.96, true),  // favourite Up at 0.96, won
            (0.04, true),  // favourite Down at 0.96, lost
            (0.93, true),  // 0.92–0.95, won
            (0.995, true), // ≥ 0.99, won
            (1.0, true),   // a certain forecast lands in the top bucket
            (0.6, false),  // 0.50–0.80, lost
        ];
        let c = calibrate(30, Some(Asset::Btc), &points);
        assert_eq!(c.n, 6);
        let b = |label: &str| c.buckets.iter().find(|b| b.label == label).unwrap().clone();
        let mid = b("0.95–0.97");
        assert_eq!(mid.n, 2);
        assert!((mid.mean_predicted.unwrap() - 0.96).abs() < 1e-12);
        assert_eq!(mid.realized, Some(0.5));
        assert_eq!(b("0.92–0.95").n, 1);
        assert_eq!(b("≥ 0.99").n, 2);
        assert_eq!(b("0.50–0.80").realized, Some(0.0));
        assert_eq!(b("0.80–0.92").n, 0);
        assert_eq!(b("0.80–0.92").ci_lo, None);
        let brier = (0.04f64.powi(2) + 0.96f64.powi(2) + 0.07f64.powi(2) + 0.005f64.powi(2)
            + 0.0
            + 0.36)
            / 6.0;
        assert!((c.brier.unwrap() - brier).abs() < 1e-12);
    }

    #[test]
    fn wilson_interval() {
        assert_eq!(wilson(0, 0), None);
        let (lo, hi) = wilson(95, 100).unwrap();
        assert!((lo - 0.8882).abs() < 1e-4 && (hi - 0.9785).abs() < 1e-4, "{lo} {hi}");
        let (lo, hi) = wilson(10, 10).unwrap();
        assert!((hi - 1.0).abs() < 1e-12 && (lo - 0.7225).abs() < 1e-4, "{lo} {hi}");
    }
}

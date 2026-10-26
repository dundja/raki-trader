//! Postgres store: sessions, windows, per-second series, feed gaps and the control audit log.
//!
//! All SQL is `&'static str` with runtime binding, so no offline `.sqlx` data is needed.
//! Enums are stored as their serde strings (`"btc"`, `"Up"`, `"timed_out"`, ...).

use anyhow::{Context, Result};
use raki_core::{
    Asset, Outcome, RecordingStats, SeriesRow, Source, TradingMode, WindowRecord, WindowStatus,
};
use rust_decimal::Decimal;
use serde::{Serialize, de::DeserializeOwned};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Postgres, QueryBuilder, Row};
use uuid::Uuid;

/// Windows needed to pass the M1 gate.
const GATE_TARGET_WINDOWS: i64 = 500;

/// Postgres allows 65535 bind parameters per statement; a series row binds 10.
const SERIES_ROWS_PER_INSERT: usize = 5_000;

#[derive(Clone)]
pub struct Store {
    pub pool: PgPool,
}

impl Store {
    /// Connects and applies any pending migrations.
    pub async fn connect(url: &str) -> Result<Store> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await
            .context("connect to postgres")?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .context("run migrations")?;
        Ok(Store { pool })
    }

    pub async fn start_session(
        &self,
        id: Uuid,
        started_ms: i64,
        version: &str,
        host: &str,
    ) -> Result<()> {
        sqlx::query("insert into sessions (id, started_ms, version, host) values ($1, $2, $3, $4)")
            .bind(id)
            .bind(started_ms)
            .bind(version)
            .bind(host)
            .execute(&self.pool)
            .await
            .context("insert session")?;
        Ok(())
    }

    pub async fn end_session(&self, id: Uuid, ended_ms: i64, reason: &str) -> Result<()> {
        sqlx::query("update sessions set ended_ms = $2, end_reason = $3 where id = $1")
            .bind(id)
            .bind(ended_ms)
            .bind(reason)
            .execute(&self.pool)
            .await
            .context("end session")?;
        Ok(())
    }

    /// Inserts or updates a window; the record is the full latest state.
    ///
    /// The first `session_id` is kept. `coverage_secs` merges as `GREATEST(existing, excluded)`:
    /// the engine's per-window counter is monotonic within a session, so a restart mid-window
    /// loses at most that window's earlier count. This is accepted.
    pub async fn upsert_window(&self, w: &WindowRecord) -> Result<()> {
        let session_id =
            Uuid::parse_str(&w.session_id).context("window session_id is not a uuid")?;
        sqlx::query(
            "insert into windows (slug, asset, start_ts, end_ts, event_id, condition_id, up_token, \
             down_token, price_to_beat_ours, final_twap_ours, outcome_ours, price_to_beat_gamma, \
             final_price_gamma, twap_match, winner, resolved_at, resolution_seen_ms, twap_source, \
             coverage_secs, status, session_id, updated_ms) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
             $18, $19, $20, $21, $22) \
             on conflict (slug) do update set \
             asset = excluded.asset, start_ts = excluded.start_ts, end_ts = excluded.end_ts, \
             event_id = excluded.event_id, condition_id = excluded.condition_id, \
             up_token = excluded.up_token, down_token = excluded.down_token, \
             price_to_beat_ours = excluded.price_to_beat_ours, \
             final_twap_ours = excluded.final_twap_ours, outcome_ours = excluded.outcome_ours, \
             price_to_beat_gamma = excluded.price_to_beat_gamma, \
             final_price_gamma = excluded.final_price_gamma, twap_match = excluded.twap_match, \
             winner = excluded.winner, resolved_at = excluded.resolved_at, \
             resolution_seen_ms = excluded.resolution_seen_ms, twap_source = excluded.twap_source, \
             coverage_secs = greatest(windows.coverage_secs, excluded.coverage_secs), \
             status = excluded.status, updated_ms = excluded.updated_ms",
        )
        .bind(&w.slug)
        .bind(enum_to_str(&w.asset)?)
        .bind(w.start_ts)
        .bind(w.end_ts)
        .bind(&w.event_id)
        .bind(&w.condition_id)
        .bind(&w.up_token)
        .bind(&w.down_token)
        .bind(w.price_to_beat_ours)
        .bind(w.final_twap_ours)
        .bind(opt_enum_to_str(&w.outcome_ours)?)
        .bind(w.price_to_beat_gamma)
        .bind(w.final_price_gamma)
        .bind(w.twap_match)
        .bind(opt_enum_to_str(&w.winner)?)
        .bind(w.resolved_at)
        .bind(w.resolution_seen_ms)
        .bind(opt_enum_to_str(&w.twap_source)?)
        .bind(w.coverage_secs)
        .bind(enum_to_str(&w.status)?)
        .bind(session_id)
        .bind(w.updated_ms)
        .execute(&self.pool)
        .await
        .with_context(|| format!("upsert window {}", w.slug))?;
        Ok(())
    }

    /// Inserts series rows with multi-row `INSERT`s; rows that already exist are left alone.
    /// Very large batches are split to stay under Postgres' bind-parameter limit.
    pub async fn insert_series(&self, rows: &[SeriesRow]) -> Result<()> {
        for chunk in rows.chunks(SERIES_ROWS_PER_INSERT) {
            let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
                "insert into window_series (slug, ts, twap, chainlink, binance_mid_usd, \
                 coinbase_mid, up_bid, up_ask, down_bid, down_ask) ",
            );
            qb.push_values(chunk, |mut b, r| {
                b.push_bind(&r.slug)
                    .push_bind(r.ts)
                    .push_bind(r.twap)
                    .push_bind(r.chainlink)
                    .push_bind(r.binance_mid_usd)
                    .push_bind(r.coinbase_mid)
                    .push_bind(r.up_bid)
                    .push_bind(r.up_ask)
                    .push_bind(r.down_bid)
                    .push_bind(r.down_ask);
            });
            qb.push(" on conflict do nothing");
            qb.build()
                .execute(&self.pool)
                .await
                .context("insert series rows")?;
        }
        Ok(())
    }

    pub async fn insert_gap(
        &self,
        session: Uuid,
        source: Source,
        at_ms: i64,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(
            "insert into feed_gaps (session_id, source, at_ms, reason) values ($1, $2, $3, $4)",
        )
        .bind(session)
        .bind(source.as_str())
        .bind(at_ms)
        .bind(reason)
        .execute(&self.pool)
        .await
        .context("insert feed gap")?;
        Ok(())
    }

    // The flat signature is the contract the engine and API call against.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_audit(
        &self,
        at_ms: i64,
        action: &str,
        ok: bool,
        from: TradingMode,
        to: TradingMode,
        reason: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "insert into control_audit (at_ms, action, ok, from_mode, to_mode, reason, error) \
             values ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(at_ms)
        .bind(action)
        .bind(ok)
        .bind(enum_to_str(&from)?)
        .bind(enum_to_str(&to)?)
        .bind(reason)
        .bind(error)
        .execute(&self.pool)
        .await
        .context("insert control audit")?;
        Ok(())
    }

    /// Newest first. `before_start` is an exclusive upper bound on `start_ts` for paging.
    pub async fn list_windows(
        &self,
        asset: Option<Asset>,
        limit: i64,
        before_start: Option<i64>,
    ) -> Result<Vec<WindowRecord>> {
        sqlx::query(
            "select * from windows \
             where ($1::text is null or asset = $1) and ($2::bigint is null or start_ts < $2) \
             order by start_ts desc limit $3",
        )
        .bind(asset.map(|a| a.as_str()))
        .bind(before_start)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("list windows")?
        .iter()
        .map(window_from_row)
        .collect()
    }

    pub async fn get_window(&self, slug: &str) -> Result<Option<WindowRecord>> {
        sqlx::query("select * from windows where slug = $1")
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .context("get window")?
            .as_ref()
            .map(window_from_row)
            .transpose()
    }

    /// All samples of a window, ordered by `ts`.
    pub async fn get_series(&self, slug: &str) -> Result<Vec<SeriesRow>> {
        let rows = sqlx::query("select * from window_series where slug = $1 order by ts")
            .bind(slug)
            .fetch_all(&self.pool)
            .await
            .context("get series")?;
        rows.iter()
            .map(|r| {
                Ok(SeriesRow {
                    slug: r.try_get("slug")?,
                    ts: r.try_get("ts")?,
                    twap: r.try_get("twap")?,
                    chainlink: r.try_get("chainlink")?,
                    binance_mid_usd: r.try_get("binance_mid_usd")?,
                    coinbase_mid: r.try_get("coinbase_mid")?,
                    up_bid: r.try_get::<Option<Decimal>, _>("up_bid")?,
                    up_ask: r.try_get::<Option<Decimal>, _>("up_ask")?,
                    down_bid: r.try_get::<Option<Decimal>, _>("down_bid")?,
                    down_ask: r.try_get::<Option<Decimal>, _>("down_ask")?,
                })
            })
            .collect()
    }

    /// Progress toward the M1 gate. Open sessions count up to now.
    pub async fn recording_stats(&self) -> Result<RecordingStats> {
        let now_ms = raki_core::now_ms();
        let sessions = sqlx::query(
            "select count(*) as sessions, \
             floor(coalesce(sum(coalesce(ended_ms, $1) - started_ms), 0) / 1000)::bigint as secs \
             from sessions",
        )
        .bind(now_ms)
        .fetch_one(&self.pool)
        .await
        .context("session stats")?;
        let windows = sqlx::query(
            "select count(*) as total, \
             count(*) filter (where final_twap_ours is not null) as with_twap, \
             count(*) filter (where twap_match is true) as matched, \
             count(*) filter (where twap_match is false) as mismatched, \
             count(*) filter (where final_twap_ours is not null and twap_match is null) as pending \
             from windows",
        )
        .fetch_one(&self.pool)
        .await
        .context("window stats")?;
        Ok(RecordingStats {
            sessions: sessions.try_get("sessions")?,
            recorded_secs: sessions.try_get("secs")?,
            windows_total: windows.try_get("total")?,
            windows_with_twap: windows.try_get("with_twap")?,
            twap_match: windows.try_get("matched")?,
            twap_mismatch: windows.try_get("mismatched")?,
            twap_pending: windows.try_get("pending")?,
            gate_target_windows: GATE_TARGET_WINDOWS,
        })
    }
}

fn window_from_row(r: &PgRow) -> Result<WindowRecord> {
    Ok(WindowRecord {
        slug: r.try_get("slug")?,
        asset: enum_from_str::<Asset>(&r.try_get::<String, _>("asset")?)?,
        start_ts: r.try_get("start_ts")?,
        end_ts: r.try_get("end_ts")?,
        event_id: r.try_get("event_id")?,
        condition_id: r.try_get("condition_id")?,
        up_token: r.try_get("up_token")?,
        down_token: r.try_get("down_token")?,
        price_to_beat_ours: r.try_get("price_to_beat_ours")?,
        final_twap_ours: r.try_get("final_twap_ours")?,
        outcome_ours: opt_enum_from_str::<Outcome>(r.try_get("outcome_ours")?)?,
        price_to_beat_gamma: r.try_get("price_to_beat_gamma")?,
        final_price_gamma: r.try_get("final_price_gamma")?,
        twap_match: r.try_get("twap_match")?,
        winner: opt_enum_from_str::<Outcome>(r.try_get("winner")?)?,
        resolved_at: r.try_get("resolved_at")?,
        resolution_seen_ms: r.try_get("resolution_seen_ms")?,
        twap_source: opt_enum_from_str::<Source>(r.try_get("twap_source")?)?,
        coverage_secs: r.try_get("coverage_secs")?,
        status: enum_from_str::<WindowStatus>(&r.try_get::<String, _>("status")?)?,
        session_id: r.try_get::<Uuid, _>("session_id")?.to_string(),
        updated_ms: r.try_get("updated_ms")?,
    })
}

/// The serde string of a unit-variant enum.
fn enum_to_str<T: Serialize>(v: &T) -> Result<String> {
    match serde_json::to_value(v)? {
        serde_json::Value::String(s) => Ok(s),
        other => anyhow::bail!("enum did not serialize to a string: {other}"),
    }
}

fn opt_enum_to_str<T: Serialize>(v: &Option<T>) -> Result<Option<String>> {
    v.as_ref().map(enum_to_str).transpose()
}

fn enum_from_str<T: DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .with_context(|| format!("unknown stored enum value {s:?}"))
}

fn opt_enum_from_str<T: DeserializeOwned>(s: Option<String>) -> Result<Option<T>> {
    s.as_deref().map(enum_from_str).transpose()
}

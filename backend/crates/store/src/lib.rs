//! Postgres store: sessions, windows, per-second series, feed gaps and the control audit log.
//!
//! All SQL is `&'static str` with runtime binding, so no offline `.sqlx` data is needed.
//! Enums are stored as their serde strings (`"btc"`, `"Up"`, `"timed_out"`, ...).

use anyhow::{Context, Result};
use raki_core::{
    Asset, Outcome, RecordingStats, SeriesRow, Source, TradingMode, WindowRecord, WindowStatus,
};
use std::str::FromStr;
use std::time::Duration;

use rust_decimal::Decimal;
use serde::{Serialize, de::DeserializeOwned};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgRow};
use sqlx::{Postgres, QueryBuilder, Row};
use uuid::Uuid;

/// Gate windows (BTC, final TWAP from PolyBolt) needed to pass the M1 gate.
const GATE_TARGET_WINDOWS: i64 = 500;

/// A store call waits at most this long for a pooled connection: with Postgres down, calls fail
/// fast instead of hanging for sqlx's default 30 s.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);
/// Server-side cap on any one statement.
const STATEMENT_TIMEOUT: &str = "10s";

/// Postgres allows 65535 bind parameters per statement; a series row binds 10.
const SERIES_ROWS_PER_INSERT: usize = 5_000;

#[derive(Clone)]
pub struct Store {
    pub pool: PgPool,
}

impl Store {
    /// Connects and applies any pending migrations. Connections wait at most 2 s for the pool and
    /// statements run at most 10 s.
    pub async fn connect(url: &str) -> Result<Store> {
        let opts = PgConnectOptions::from_str(url)
            .context("parse database url")?
            .options([("statement_timeout", STATEMENT_TIMEOUT)]);
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .connect_with(opts)
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
        sqlx::query(
            "update sessions set ended_ms = $2, end_reason = $3, \
             last_seen_ms = greatest(last_seen_ms, $2) where id = $1",
        )
        .bind(id)
        .bind(ended_ms)
        .bind(reason)
        .execute(&self.pool)
        .await
        .context("end session")?;
        Ok(())
    }

    /// Records that the session is alive at `at_ms`.
    pub async fn heartbeat(&self, id: Uuid, at_ms: i64) -> Result<()> {
        sqlx::query(
            "update sessions set last_seen_ms = greatest(coalesce(last_seen_ms, $2), $2) \
             where id = $1",
        )
        .bind(id)
        .bind(at_ms)
        .execute(&self.pool)
        .await
        .context("session heartbeat")?;
        Ok(())
    }

    /// Closes every session that never ended (crash, kill -9, power loss) with reason `crash`,
    /// and logs the crash as a `system` gap. Call it before starting a new session.
    ///
    /// The end is the newest evidence the session was alive: its heartbeat, its latest gap or
    /// window update, capped at the next session's start and at `now_ms`. Returns
    /// `(session, ended_ms)` for every session closed.
    pub async fn close_dangling_sessions(&self, now_ms: i64) -> Result<Vec<(Uuid, i64)>> {
        let rows = sqlx::query(
            "with dangling as ( \
               select s.id, least( \
                 coalesce((select min(n.started_ms) from sessions n \
                           where n.started_ms > s.started_ms), $1), \
                 $1, \
                 greatest(s.started_ms, s.last_seen_ms, \
                   (select max(g.at_ms) from feed_gaps g where g.session_id = s.id), \
                   (select max(w.updated_ms) from windows w where w.session_id = s.id)) \
               ) as ended \
               from sessions s where s.ended_ms is null \
             ), closed as ( \
               update sessions set ended_ms = d.ended, end_reason = 'crash' \
               from dangling d where sessions.id = d.id \
               returning sessions.id, sessions.ended_ms \
             ) \
             insert into feed_gaps (session_id, source, at_ms, reason) \
             select id, 'system', ended_ms, 'crash: session ended without a clean shutdown' \
             from closed returning session_id, at_ms",
        )
        .bind(now_ms)
        .fetch_all(&self.pool)
        .await
        .context("close dangling sessions")?;
        rows.iter()
            .map(|r| Ok((r.try_get("session_id")?, r.try_get("at_ms")?)))
            .collect()
    }

    /// Logs a machine sleep inside a session: a `system` gap from `wake_ms - slept_ms` lasting
    /// `slept_ms`, and adds it to the session's slept time (excluded from recorded time).
    pub async fn record_sleep(&self, session: Uuid, wake_ms: i64, slept_ms: i64) -> Result<()> {
        let mut tx = self.pool.begin().await.context("begin sleep")?;
        sqlx::query(
            "insert into feed_gaps (session_id, source, at_ms, reason, duration_ms) \
             values ($1, 'system', $2, 'sleep', $3)",
        )
        .bind(session)
        .bind(wake_ms - slept_ms)
        .bind(slept_ms)
        .execute(&mut *tx)
        .await
        .context("insert sleep gap")?;
        sqlx::query("update sessions set slept_ms = slept_ms + $2 where id = $1")
            .bind(session)
            .bind(slept_ms)
            .execute(&mut *tx)
            .await
            .context("add slept time")?;
        tx.commit().await.context("commit sleep")?;
        Ok(())
    }

    /// Windows an earlier session left unfinished (anything but `complete`) that ended at or
    /// after `since_end_ts` or have not ended yet, newest first, at most `limit`. A restart
    /// adopts them so they still get their winner, Gamma final price and TWAP match.
    pub async fn unfinished_windows(
        &self,
        since_end_ts: i64,
        limit: i64,
    ) -> Result<Vec<WindowRecord>> {
        sqlx::query(
            "select * from windows where status <> 'complete' and end_ts >= $1 \
             order by end_ts desc limit $2",
        )
        .bind(since_end_ts)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("unfinished windows")?
        .iter()
        .map(window_from_row)
        .collect()
    }

    /// Inserts or updates a window from the engine's latest record.
    ///
    /// Nothing known is ever lost or regressed, so a restarted engine (whose fresh record does
    /// not know what an earlier session saw) or a stale retry cannot erase data:
    /// - ids, our prices and their sources, the outcome, Gamma's prices and the TWAP match take
    ///   the new value when it is known, else keep the stored one;
    /// - the winner, `resolved_at` and `resolution_seen_ms` are latched: the first value stays;
    /// - the status only moves forward (`window_status_rank`), `coverage_secs` and `updated_ms`
    ///   only grow;
    /// - the first `session_id` is kept.
    pub async fn upsert_window(&self, w: &WindowRecord) -> Result<()> {
        let session_id =
            Uuid::parse_str(&w.session_id).context("window session_id is not a uuid")?;
        sqlx::query(
            "insert into windows (slug, asset, start_ts, end_ts, event_id, condition_id, up_token, \
             down_token, price_to_beat_ours, final_twap_ours, outcome_ours, price_to_beat_gamma, \
             final_price_gamma, twap_match, winner, resolved_at, resolution_seen_ms, twap_source, \
             coverage_secs, status, session_id, updated_ms, final_twap_source) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
             $18, $19, $20, $21, $22, $23) \
             on conflict (slug) do update set \
             asset = excluded.asset, start_ts = excluded.start_ts, end_ts = excluded.end_ts, \
             event_id = coalesce(excluded.event_id, windows.event_id), \
             condition_id = coalesce(excluded.condition_id, windows.condition_id), \
             up_token = coalesce(excluded.up_token, windows.up_token), \
             down_token = coalesce(excluded.down_token, windows.down_token), \
             price_to_beat_ours = coalesce(excluded.price_to_beat_ours, windows.price_to_beat_ours), \
             final_twap_ours = coalesce(excluded.final_twap_ours, windows.final_twap_ours), \
             outcome_ours = coalesce(excluded.outcome_ours, windows.outcome_ours), \
             price_to_beat_gamma = coalesce(excluded.price_to_beat_gamma, windows.price_to_beat_gamma), \
             final_price_gamma = coalesce(excluded.final_price_gamma, windows.final_price_gamma), \
             twap_match = coalesce(excluded.twap_match, windows.twap_match), \
             twap_source = coalesce(excluded.twap_source, windows.twap_source), \
             final_twap_source = coalesce(excluded.final_twap_source, windows.final_twap_source), \
             winner = coalesce(windows.winner, excluded.winner), \
             resolved_at = coalesce(windows.resolved_at, excluded.resolved_at), \
             resolution_seen_ms = coalesce(windows.resolution_seen_ms, excluded.resolution_seen_ms), \
             coverage_secs = greatest(windows.coverage_secs, excluded.coverage_secs), \
             status = case when window_status_rank(excluded.status) >= window_status_rank(windows.status) \
               then excluded.status else windows.status end, \
             updated_ms = greatest(windows.updated_ms, excluded.updated_ms)",
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
        .bind(opt_enum_to_str(&w.final_twap_source)?)
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

    /// Progress toward the M1 gate. Open sessions count up to now; machine sleep is excluded.
    pub async fn recording_stats(&self) -> Result<RecordingStats> {
        let now_ms = raki_core::now_ms();
        let sessions = sqlx::query(
            "select count(*) as sessions, \
             floor(coalesce(sum(greatest(coalesce(ended_ms, $1) - started_ms - slept_ms, 0)), 0) \
               / 1000)::bigint as secs, \
             floor(coalesce(sum(slept_ms), 0) / 1000)::bigint as slept \
             from sessions",
        )
        .bind(now_ms)
        .fetch_one(&self.pool)
        .await
        .context("session stats")?;
        // `g` marks a gate window: BTC with a PolyBolt final TWAP (CLAUDE.md §5).
        let windows = sqlx::query(
            "with w as ( \
               select *, (asset = 'btc' and final_twap_source = 'polybolt' \
                          and final_twap_ours is not null) as g, \
                 (final_twap_ours is not null and twap_match is null \
                  and status <> 'timed_out') as awaiting, \
                 (final_twap_ours is not null and twap_match is null \
                  and status = 'timed_out') as unverified \
               from windows) \
             select count(*) filter (where status <> 'upcoming') as total, \
             count(*) filter (where final_twap_ours is not null) as with_twap, \
             count(*) filter (where twap_match is true) as matched, \
             count(*) filter (where twap_match is false) as mismatched, \
             count(*) filter (where awaiting) as awaiting, \
             count(*) filter (where unverified) as unverified, \
             count(*) filter (where g) as gate, \
             count(*) filter (where g and twap_match is true) as gate_matched, \
             count(*) filter (where g and twap_match is false) as gate_mismatched, \
             count(*) filter (where g and awaiting) as gate_awaiting, \
             count(*) filter (where g and unverified) as gate_unverified \
             from w",
        )
        .fetch_one(&self.pool)
        .await
        .context("window stats")?;
        Ok(RecordingStats {
            sessions: sessions.try_get("sessions")?,
            recorded_secs: sessions.try_get("secs")?,
            slept_secs: sessions.try_get("slept")?,
            windows_total: windows.try_get("total")?,
            windows_with_twap: windows.try_get("with_twap")?,
            twap_match: windows.try_get("matched")?,
            twap_mismatch: windows.try_get("mismatched")?,
            twap_awaiting: windows.try_get("awaiting")?,
            twap_unverified: windows.try_get("unverified")?,
            gate_windows: windows.try_get("gate")?,
            gate_twap_match: windows.try_get("gate_matched")?,
            gate_twap_mismatch: windows.try_get("gate_mismatched")?,
            gate_twap_awaiting: windows.try_get("gate_awaiting")?,
            gate_twap_unverified: windows.try_get("gate_unverified")?,
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
        final_twap_source: opt_enum_from_str::<Source>(r.try_get("final_twap_source")?)?,
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

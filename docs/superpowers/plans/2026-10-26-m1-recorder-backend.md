# M1 Recorder Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust process (`traderd`) that, while running on the user's Mac, records every relevant Polymarket / Chainlink-TWAP / Binance / Coinbase message losslessly to disk. It also tracks each BTC/ETH/SOL 5-minute window through to resolution, checks our recorded TWAP against Polymarket's settlement price, and serves health, live state and pause/resume/kill controls over a local HTTP and WebSocket API.

**Architecture:** A Cargo workspace in `backend/`, with seven crates:
- `raki-core`: shared types and DTOs.
- `raki-recorder`: raw zstd NDJSON writer and reader.
- `raki-feeds`: pure parsers, WebSocket feed runners, REST clients.
- `raki-engine`: pure state machine with no I/O (books, oracle, windows, series, control).
- `raki-store`: Postgres via sqlx.
- `raki-api`: axum.
- `traderd`: the binary. It wires everything together and runs the health monitors.

Feeds push every raw frame to the recorder *before* parsing, and push parsed `FeedEvent`s to the engine. The engine emits `EngineOutput`s (rows to store, REST requests to run, desired CLOB tokens), which the `traderd` shell executes.

**Tech Stack:**
- Rust 1.99 (edition 2024, `rust-version = "1.94"`), tokio 1.54.
- WebSockets: tokio-tungstenite 0.30 (rustls). HTTP: reqwest 0.13 (rustls).
- API: axum 0.8.9 (`ws`) and tower-http 0.7 (`cors`).
- Database: sqlx 0.9 (postgres, runtime-tokio, tls-rustls-aws-lc-rs, rust_decimal, uuid), Postgres 17 in Docker.
- Data: zstd 0.14, serde/serde_json (`raw_value`), rust_decimal 1.43 (`serde-with-str`).
- Other: secrecy 0.10, tracing, toml, chrono, uuid v7, rsntp 4.1, libc, ts-rs (TypeScript export).
- Tests: wiremock 0.6, testcontainers-modules 0.15 (postgres 17-alpine).

**Spec:**
- `CLAUDE.md` (§2 market facts, §4 architecture, §5 M1 gate, §6 safety checks, §7 secrets, §8 local hosting).
- Wire-level facts: `docs/research/2026-10-26-m1-protocol-notes.md`. Its Cross-check section wins over the topic sections.
- Fixtures: `backend/fixtures/README.md`.

## Global Constraints

- Money and book prices/sizes are `rust_decimal::Decimal`. They are parsed from the wire **strings** with `rust_decimal::serde::str`; never parse a price through f64. Oracle and TWAP values are `f64`, because Polymarket settles on the f64 `value`.
- Every recorded record carries `recv_ns` (local `SystemTime` epoch ns). Source timestamps are kept separately. All times are UTC.
- Raw frames are written to the recorder **before** parsing. A parse failure never drops the raw frame.
- No secrets in the repo, logs or `Debug` output. PolyBolt credentials come only from the env vars `POLY_API_KEY`, `POLY_API_SECRET` and `POLY_API_PASSPHRASE`, held as `secrecy::SecretString`. Never read `.env*` files.
- Never place orders. M1 has no order code. The control state machine only gates future trading.
- First line of `main`: `rustls::crypto::aws_lc_rs::default_provider().install_default()`. This avoids the ring/aws-lc-rs panic described in the protocol notes §2.
- No library-like `User-Agent` header on HTTP requests; reqwest's default (none) is fine. CLOB returned 403 to `Python-urllib`.
- Gamma requests add the cache-buster `?cb=<now_ms>`. Responses are CDN-cached for up to 300 s.
- The Data API `/v2/resolutions` is the resolution authority; the CLOB WebSocket `market_resolved` event is a second signal. Once resolved, the state is latched and never goes back. CLOB REST `/markets` must not be used to decide resolution.
- Outcome mapping goes by the outcome **string** (`"Up"`/`"Down"`). Assert that index 0 is `"Up"`; if it isn't, log an error and skip that window.
- Recording runs only while the app runs. Every app start opens a new **session** (uuid v7), recorded in Postgres and in the raw stream.
- Default local endpoints:
  - API `127.0.0.1:8787`.
  - Postgres `postgres://raki:raki@127.0.0.1:5433/raki`. These are local dev credentials, not secrets.
  - Dashboard origin `http://localhost:3000`.
- Clippy is clean with `-D warnings`. `cargo fmt` is applied.

## Review Focus

1. **The Mac sleeps mid-run.** tokio timers freeze and sockets die silently. Expected behaviour: a `Sleep` record, a gap per feed, a forced reconnect of all feeds, and safety checks failing until feeds are fresh. Tests: Task 8 `sleep_detector_flags_drift`, Task 10 smoke step.
- **The CLOB server closes with 1000 right after `market_resolved`**, or the connection drops at any time. Expected: reconnect within backoff, then resubscribe *all* desired tokens in one initial frame with `custom_feature_enabled:true`. A `Close` record and a gap are written. Test: Task 4 `clob_reconnects_and_resubscribes_after_server_close`.
- **Gamma is cached or late.** `priceToBeat`/`finalPrice` are absent for minutes, and responses may be up to 5 minutes stale. Expected: the window keeps its own TWAP values, keeps polling Gamma every 30 s for up to 30 minutes, then marks `timed_out` without crashing. Test: Task 6 `gamma_final_polling_times_out`.
- **Disk-full, or the process is killed.** The raw file is left without a finished zstd frame. Expected: the reader returns every flushed line plus `truncated = true`, and the next start writes a **new** file instead of appending. Tests: Task 2 `reader_recovers_flushed_lines_from_truncated_file`, `writer_never_appends_to_existing_file`.
- **The recorder channel is full** (CLOB bursts about 400 frames/s). Expected: `try_send` failure increments `dropped`, and the feed never blocks or panics. The health snapshot shows `dropped > 0` and the recorder safety check fails. Test: Task 2 `record_tx_counts_drops_when_full`.

---

## File Structure

```
docker-compose.yml                 # postgres:17-alpine on 5433, volume raki-pg
Makefile                           # db-up, db-down, backend-test, backend-run, frontend-dev, dev, test
config/traderd.toml                # non-secret runtime config (committed)
backend/
  Cargo.toml                       # [workspace] members, [workspace.package], [workspace.dependencies]
  .cargo/config.toml               # TS_RS_EXPORT_DIR -> ../frontend/src/api/generated (relative=true)
  rust-toolchain.toml              # channel = "stable"
  fixtures/                        # real captures (already present; see README)
  crates/core/src/{lib.rs, asset.rs, window.rs, time.rs, source.rs, events.rs, dto.rs}
  crates/recorder/src/{lib.rs, record.rs, writer.rs, reader.rs, task.rs}
  crates/feeds/src/{lib.rs, status.rs, backoff.rs, ws_conn.rs,
                    parse/{mod.rs, clob.rs, rtds.rs, polybolt.rs, binance.rs, coinbase.rs},
                    runners/{mod.rs, clob.rs, rtds.rs, polybolt.rs, binance.rs, coinbase.rs},
                    rest/{mod.rs, gamma.rs, data_api.rs, geoblock.rs}}
  crates/feeds/tests/{parsers.rs, runners.rs, rest.rs}
  crates/engine/src/{lib.rs, book.rs, oracle.rs, cex.rs, windows.rs, series.rs, control.rs, engine.rs}
  crates/store/src/lib.rs, crates/store/migrations/0001_init.sql, crates/store/tests/store.rs
  crates/api/src/{lib.rs, routes.rs, ws.rs}, crates/api/tests/api.rs
  crates/traderd/src/{main.rs, config.rs, health/{mod.rs, clock.rs, sleep.rs, geoblock.rs}, shell.rs, shutdown.rs}
```

---

### Task 1: Workspace, core types, dev infrastructure

**Files:**
- Create: `docker-compose.yml`, `Makefile`, `config/traderd.toml`, `backend/Cargo.toml`, `backend/.cargo/config.toml`, `backend/rust-toolchain.toml`, `backend/crates/core/**`.
- Create a compiling skeleton for **every** crate listed in File Structure:
  - each with its `Cargo.toml` and all of its dependencies;
  - `lib.rs` (or `main.rs`) with `pub mod` declarations;
  - empty module files (`// implemented in Task N`).

  Later tasks can then run in parallel without touching `backend/Cargo.toml` or each other's `lib.rs`.
- Test: `backend/crates/core/src/window.rs` (unit tests), `backend/crates/core/src/dto.rs` (export test).

**Interfaces — Produces (all in `raki_core`, re-exported at crate root):**

```rust
// asset.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Asset { Btc, Eth, Sol }
impl Asset {
    pub const ALL: [Asset; 3];
    pub fn as_str(self) -> &'static str;            // "btc"
    pub fn rtds_symbol(self) -> &'static str;       // "btc/usd"
    pub fn polybolt_symbol(self) -> &'static str;   // "btcusd"
    pub fn binance_symbol(self) -> &'static str;    // "btcusdt" (lowercase stream name)
    pub fn coinbase_product(self) -> &'static str;  // "BTC-USD"
    pub fn from_any(s: &str) -> Option<Asset>;      // accepts all of the above forms, case-insensitive
}
// window.rs
pub const WINDOW_SECS: i64 = 300;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WindowId { pub asset: Asset, pub start: i64 }   // start: unix seconds, multiple of 300
impl WindowId {
    pub fn containing(asset: Asset, unix_s: i64) -> WindowId;  // floor to 300
    pub fn next(self) -> WindowId;
    pub fn end(self) -> i64;                                   // start + 300
    pub fn slug(self) -> String;                               // "btc-updown-5m-1793031000"
    pub fn parse_slug(s: &str) -> Option<WindowId>;            // rejects non-multiple-of-300 and unknown asset
    pub fn secs_left(self, now_ms: i64) -> f64;                // (end*1000 - now_ms)/1000, may be negative
}
// time.rs
pub fn now_ns() -> i64;  pub fn now_ms() -> i64;               // SystemTime since UNIX_EPOCH
// source.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Source { ClobMarket, RtdsLegacy, Polybolt, Binance, Coinbase, Gamma, DataApi, Geoblock, System }
impl Source { pub fn as_str(self) -> &'static str; }           // matches serde name
// events.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)] #[serde(rename_all="snake_case")]
pub enum Side { Buy, Sell }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum Outcome { Up, Down }                                   // serialized "Up"/"Down"
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)] #[serde(rename_all="snake_case")]
pub enum OracleKind { Chainlink, Twap60 }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)] #[serde(rename_all="snake_case")]
pub enum Venue { Binance, Coinbase }
#[derive(Clone, Debug, PartialEq)] pub struct Level { pub price: Decimal, pub size: Decimal }
#[derive(Clone, Debug, PartialEq)] pub struct PriceChangeEntry { pub asset_id: String, pub price: Decimal, pub size: Decimal, pub side: Side, pub best_bid: Option<Decimal>, pub best_ask: Option<Decimal> }
#[derive(Clone, Debug, PartialEq)] pub struct GammaWindow {
    pub slug: String, pub event_id: String, pub start: i64, pub end: i64,
    pub condition_id: String, pub up_token: String, pub down_token: String,
    pub price_to_beat: Option<f64>, pub final_price: Option<f64>, pub closed: bool,
    pub twap_lookback_s: Option<i64>, pub fee_rate: Option<f64>, pub tick_size: Option<f64> }
#[derive(Clone, Debug, PartialEq)]
pub enum FeedEvent {
    Book { asset_id: String, market: String, ts_ms: i64, bids: Vec<Level>, asks: Vec<Level> },
    PriceChange { market: String, ts_ms: i64, changes: Vec<PriceChangeEntry> },
    BestBidAsk { asset_id: String, ts_ms: i64, best_bid: Option<Decimal>, best_ask: Option<Decimal> },
    LastTrade { asset_id: String, ts_ms: i64, price: Decimal, size: Decimal, side: Side },
    TickSize { asset_id: String, ts_ms: i64, new_tick: Decimal },
    MarketResolved { condition_id: String, winning_asset_id: String, winning_outcome: String, recv_ms: i64 },
    Oracle { asset: Asset, kind: OracleKind, source: Source, ts_ms: i64, value: f64 },
    CexQuote { venue: Venue, asset: Asset, bid: Decimal, ask: Decimal, recv_ms: i64 },
    UsdtUsd { price: Decimal, recv_ms: i64 },
    FeedReset { source: Source },                     // emitted by a runner on every (re)connect
    GammaFetched { slug: String, window: Option<GammaWindow>, recv_ms: i64 },   // None = 404
    GammaFailed { slug: String },
    Resolution { condition_id: String, winner: Option<Outcome>, resolved_at: Option<i64> }, // None = not yet
}
```

`dto.rs`: every type the API returns.
- All of them derive `Serialize, Deserialize, Clone, Debug, PartialEq, TS` with `#[ts(export)]`.
- `i64` and `u64` fields get `#[ts(type = "number")]`; ts-rs would otherwise emit `bigint`, which JSON can't carry.
- `Decimal` fields get `#[ts(type = "string")]` (or `"string | null"` when optional) and `#[serde(with = "rust_decimal::serde::str")]` (or `str_option`).

```rust
#[serde(rename_all = "snake_case")] pub enum WindowStatus { Upcoming, Live, Ended, Resolved, Complete, TimedOut }
pub struct WindowRecord {
    pub slug: String, pub asset: Asset, pub start_ts: i64, pub end_ts: i64,
    pub event_id: Option<String>, pub condition_id: Option<String>, pub up_token: Option<String>, pub down_token: Option<String>,
    pub price_to_beat_ours: Option<f64>, pub final_twap_ours: Option<f64>, pub outcome_ours: Option<Outcome>,
    pub price_to_beat_gamma: Option<f64>, pub final_price_gamma: Option<f64>, pub twap_match: Option<bool>,
    pub winner: Option<Outcome>, pub resolved_at: Option<i64>, pub resolution_seen_ms: Option<i64>,
    pub twap_source: Option<Source>, pub coverage_secs: i32, pub status: WindowStatus, pub session_id: String, pub updated_ms: i64 }
pub struct SeriesRow { pub slug: String, pub ts: i64, pub twap: Option<f64>, pub chainlink: Option<f64>,
    pub binance_mid_usd: Option<f64>, pub coinbase_mid: Option<f64>,
    pub up_bid: Option<Decimal>, pub up_ask: Option<Decimal>, pub down_bid: Option<Decimal>, pub down_ask: Option<Decimal> }
pub struct FeedStatusDto { pub source: Source, pub enabled: bool, pub connected: bool, pub last_msg_ms: Option<i64>,
    pub msgs_total: u64 /*ts type number*/, pub msgs_per_sec: f64, pub reconnects: u32, pub last_error: Option<String>, pub stale: bool }
pub struct ClockStatus { pub offset_ms: Option<f64>, pub ok: bool, pub checked_ms: Option<i64>, pub max_offset_ms: f64 }
pub struct GeoStatus { pub blocked: Option<bool>, pub country: Option<String>, pub checked_ms: Option<i64> }
pub struct RecorderStatus { pub file: Option<String>, pub records: u64, pub bytes: u64, pub dropped: u64, pub last_flush_ms: Option<i64> }
pub struct SafetyChecks { pub feeds_fresh: bool, pub clock_ok: bool, pub geoblock_ok: bool, pub recorder_ok: bool, pub all_ok: bool, pub reasons: Vec<String> }
pub struct HealthSnapshot { pub feeds: Vec<FeedStatusDto>, pub clock: ClockStatus, pub geoblock: GeoStatus, pub recorder: RecorderStatus,
    pub sleep_events: u32, pub book_resyncs: u32, pub safety: SafetyChecks }
#[serde(rename_all = "snake_case")] pub enum TradingMode { Running, Paused, Killed }
#[serde(rename_all = "snake_case")] pub enum StopKind { Startup, Manual, Safety, LossCap, ConfigHalt }
pub struct ControlState { pub mode: TradingMode, pub stop_kind: Option<StopKind>, pub reason: Option<String>, pub needs_ack: bool, pub since_ms: i64 }
pub struct LiveWindow { pub asset: Asset, pub slug: String, pub start_ts: i64, pub end_ts: i64, pub secs_left: f64,
    pub price_to_beat: Option<f64>, pub twap: Option<f64>, pub twap_ts_ms: Option<i64>, pub chainlink: Option<f64>,
    pub binance_mid_usd: Option<f64>, pub coinbase_mid: Option<f64>,
    pub up_bid: Option<Decimal>, pub up_ask: Option<Decimal>, pub down_bid: Option<Decimal>, pub down_ask: Option<Decimal> }
pub struct SessionInfo { pub id: String, pub started_ms: i64, pub version: String }
pub struct LiveSnapshot { pub generated_ms: i64, pub session: SessionInfo, pub health: HealthSnapshot, pub control: ControlState, pub windows: Vec<LiveWindow> }
pub struct RecordingStats { pub sessions: i64, pub recorded_secs: i64, pub windows_total: i64, pub windows_with_twap: i64,
    pub twap_match: i64, pub twap_mismatch: i64, pub twap_pending: i64, pub gate_target_windows: i64 /* = 500 */ }
pub struct ControlRequest { pub reason: Option<String>, pub acknowledge: Option<bool> }
pub struct ControlResponse { pub ok: bool, pub state: ControlState, pub error: Option<String> }
```

**Config (`config/traderd.toml`, committed; parsed in Task 10):**

```toml
data_dir = "data"
database_url = "postgres://raki:raki@127.0.0.1:5433/raki"
api_bind = "127.0.0.1:8787"
cors_origin = "http://localhost:3000"
assets = ["btc", "eth", "sol"]
[feeds]
clob = true
rtds_legacy = true
polybolt = "auto"
binance = true
coinbase = true
[health]
clock_max_offset_ms = 50
ntp_servers = ["time.apple.com", "time.cloudflare.com", "pool.ntp.org"]
ntp_interval_s = 60
geoblock_interval_s = 300
stale_ms = { clob_market = 10000, rtds_legacy = 3000, polybolt = 3000, binance = 5000, coinbase = 5000 }
```

`polybolt = "auto"` means enabled only when all three env vars are set.

**Steps:**
- [ ] **Step 1: Write the failing tests.** Add the following to `crates/core/src/window.rs`:
  - `containing_floors_to_300`: `WindowId::containing(Btc, 1793031299).start == 1793031000`, and `containing(Btc, 1793031300).start == 1793031300`.
  - `slug_roundtrip`: `slug() == "btc-updown-5m-1793031000"`, and `parse_slug` of that equals the id.
  - `parse_slug_rejects`: `"btc-updown-5m-1793031001"`, `"doge-updown-5m-1793031000"`, `"btc-updown-15m-1793031000"` and `"garbage"` all give `None`.
  - `secs_left_math`: at `now_ms = 1793031270000`, `secs_left == 30.0`.
  - `asset_symbols`: `Btc.rtds_symbol()=="btc/usd"`, `polybolt_symbol()=="btcusd"`, `coinbase_product()=="BTC-USD"`; `from_any("BTCUSDT")==Some(Btc)`, `from_any("sol/usd")==Some(Sol)`.
- [ ] **Step 2: Run them to make sure they fail.** Run `cd backend && cargo test -p raki-core`. Expected: compile errors or FAIL.
- [ ] **Step 3: Implement the core modules.**
  - Implement `raki_core` as specified above.
  - Write `docker-compose.yml`: service `db`, image `postgres:17-alpine`, env `POSTGRES_USER/PASSWORD/DB=raki`, ports `5433:5432`, named volume `raki-pg`, healthcheck `pg_isready -U raki`.
  - Write the `Makefile` targets:
    - `db-up`: `docker compose up -d db` and wait for healthy.
    - `backend-test`: `cd backend && cargo test --workspace`.
    - `backend-run`: `caffeinate -is cargo run --release --manifest-path backend/Cargo.toml -p traderd -- --config config/traderd.toml`.
    - `frontend-dev`: `cd frontend && pnpm dev`.
    - `dev`: db-up, then backend-run and frontend-dev concurrently.
    - `test`: backend-test plus `cd frontend && pnpm test`.
  - Write the skeletons of all the other crates, with dependencies.
- [ ] **Step 4: Check that the workspace builds and core passes.** Run `cd backend && cargo test -p raki-core && cargo build --workspace && cargo clippy --workspace -- -D warnings`. Expected: PASS, no warnings.
- [ ] **Step 5: Check type export and the database container.**
  - Run `cd backend && cargo test -p raki-core export_bindings`. Expected: `.ts` files appear in `frontend/src/api/generated/` (e.g. `LiveSnapshot.ts`), and `LiveSnapshot.ts` shows `generated_ms: number`.
  - Run `make db-up && docker compose ps`. Expected: `db` is healthy.
- [ ] **Step 6: Commit.** `git add -A docker-compose.yml Makefile config backend frontend/src/api/generated && git commit -m "feat(m1): workspace, core types, dev infra"`

---

### Task 2: Raw recorder (`raki-recorder`)

**Files:** `backend/crates/recorder/src/{record.rs, writer.rs, reader.rs, task.rs}`. Tests go in each module's `#[cfg(test)]`.

**Interfaces:**
- Consumes: `raki_core::{Source, now_ns}`.
- Produces:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all="snake_case")]
pub enum RecordKind { Text, Open, Close, Error, Send, Gap, SessionStart, SessionEnd, Sleep, Http }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record { pub t: i64 /*recv_ns*/, pub s: Source, pub c: u32 /*connection seq*/, pub k: RecordKind, pub d: String /*exact text*/ }
pub struct RawWriter;   // impl:
impl RawWriter {
    pub fn new(dir: PathBuf, session: Uuid) -> RawWriter;   // lazy-opens files
    pub fn write(&mut self, rec: &Record) -> io::Result<()>; // one JSON line; rotates when UTC hour of rec.t changes
    pub fn flush(&mut self) -> io::Result<()>;              // zstd ZSTD_e_flush + file flush
    pub fn finish(&mut self) -> io::Result<()>;             // end frame, sync_all, close
    pub fn current_file(&self) -> Option<PathBuf>;
}
pub struct ReadOutcome { pub records: Vec<Record>, pub truncated: bool, pub bad_lines: usize }
pub fn read_file(path: &Path) -> io::Result<ReadOutcome>;
#[derive(Default)] pub struct RecorderStats { pub records: AtomicU64, pub bytes: AtomicU64, pub dropped: AtomicU64, pub last_flush_ms: AtomicI64, pub current_file: Mutex<Option<String>> }
#[derive(Clone)] pub struct RecordTx { /* mpsc::Sender<Record>, Arc<RecorderStats> */ }
impl RecordTx { pub fn send(&self, rec: Record); /* try_send; on Full/Closed increments stats.dropped */ pub fn stats(&self) -> Arc<RecorderStats>; }
pub fn spawn_recorder(dir: PathBuf, session: Uuid, capacity: usize) -> (RecordTx, tokio::task::JoinHandle<io::Result<()>>);
```

- **File path:** `<dir>/raw/YYYY-MM-DD/HH-<session first 8 hex>-<n>.ndjson.zst`.
  - `n` starts at 0 and increments if that path already exists. **Never append** to an existing file.
  - zstd level 3, `include_checksum(true)`.
- **Recorder task:**
  - Writes records as they arrive and flushes every 1 s (and at shutdown).
  - On channel close it calls `finish()` and returns.
  - Updates `stats.records` and `bytes` (uncompressed line bytes), `last_flush_ms` and `current_file`.
- **Reader:** decodes a possibly-truncated file.
  - An `UnexpectedEof` from zstd sets `truncated = true` and keeps the lines decoded so far.
  - A final line with no trailing `\n` is dropped.
  - Lines that fail JSON parsing are counted in `bad_lines`.

- [ ] **Step 1: Write the failing tests:**
  - `roundtrip_preserves_exact_text`: write 3 records whose `d` holds quotes, newlines, unicode and a 100 KB string, call `finish`, then `read_file`. The records are equal and `truncated == false`.
  - `rotates_on_utc_hour_change`: records at `t` 2026-10-26T10:59:59Z and 11:00:00Z produce two files named `10-…` and `11-…` under `raw/2026-10-26/`.
  - `reader_recovers_flushed_lines_from_truncated_file`: write 100 records with `flush()` after record 60, write 40 more, `flush()` again, then truncate the file by 7 bytes without finish. `read_file` gives ≥ 60 records, `truncated == true`, and all returned records equal their originals.
  - `writer_never_appends_to_existing_file`: create `HH-<s8>-0.ndjson.zst` beforehand. The writer creates `-1`.
  - `record_tx_counts_drops_when_full` (tokio): create a capacity-2 channel with no consumer progress (hold the receiver unpolled by using an internal constructor `RecordTx::for_test(cap)`), then send 5. `stats.dropped == 3`.
  - `spawned_recorder_flushes_and_finishes`: spawn, send 10, drop the tx, await the handle. The file decodes with 10 records and `truncated == false`.
- [ ] **Step 2: Run them to make sure they fail.** Run `cargo test -p raki-recorder`. Expected: FAIL.
- [ ] **Step 3: Implement the writer, reader and task** as specified.
- [ ] **Step 4: Run the tests to make sure they pass.** Run `cargo test -p raki-recorder`. Expected: PASS.
- [ ] **Step 5: Commit.** `git add backend/crates/recorder && git commit -m "feat(recorder): lossless hourly zstd ndjson recorder"`

---

### Task 3: Pure frame parsers (`raki_feeds::parse`)

**Files:** `backend/crates/feeds/src/parse/{clob.rs, rtds.rs, polybolt.rs, binance.rs, coinbase.rs}`, `backend/crates/feeds/tests/parsers.rs`.

**Interfaces:**
- Consumes: `raki_core::{FeedEvent, Level, PriceChangeEntry, Side, Asset, OracleKind, Source, Venue}`.
- Produces: each parser is `fn(&str, recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError>`. Control or ack frames return `Ok(vec![])`.

```rust
#[derive(Debug, thiserror::Error)] pub enum ParseError { #[error("json: {0}")] Json(String), #[error("unexpected: {0}")] Unexpected(String) }
pub fn parse_clob(text: &str, recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError>;
pub fn parse_rtds(text: &str, recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError>;
pub fn parse_polybolt(text: &str, recv_ms: i64) -> Result<PolyboltFrame, ParseError>;
pub enum PolyboltFrame { Events(Vec<FeedEvent>), Authed, Subscribed { channel: String }, Error { code: String }, Pong, Other, SeqGap { channel: String, expected: u64, got: u64 } }
pub struct PolyboltSeq { /* per-channel last seq */ } impl PolyboltSeq { pub fn check(&mut self, channel: &str, seq: u64) -> Option<(u64,u64)>; pub fn reset(&mut self); }
pub fn parse_binance(text: &str, recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError>;
pub fn parse_coinbase(text: &str, recv_ms: i64) -> Result<Vec<FeedEvent>, ParseError>;
```

**Parser rules** (from the protocol notes):

| Parser | Rules |
|---|---|
| **CLOB** | Text `PONG` → `[]`. A frame starting with `[` is an array of `book`. Each object's `event_type` is one of: `book` → `Book`; `price_change` → one `PriceChange` (size `"0"` kept as 0); `best_bid_ask` → `BestBidAsk` (empty or absent → `None`); `last_trade_price` → `LastTrade`; `tick_size_change` → `TickSize`; `market_resolved` → `MarketResolved` (accept `assets_ids` or `asset_ids`; `condition_id = market`); `new_market` → `[]`; unknown → `[]`. `timestamp` is a ms string. |
| **RTDS legacy** | `""` → `[]`. Topic `crypto_prices_twap_sixty` → `Twap60`. Topic `crypto_prices_chainlink`, *or* topic `crypto_prices` with `type == "subscribe"` and a symbol containing `/`, → `Chainlink`. Topic `crypto_prices` with a `*usdt` symbol (Binance relay) → `[]`, since we take Binance directly. Snapshots (`payload.data[]`) emit one event per point. `source = RtdsLegacy`, `value` = the f64 `value`. |
| **PolyBolt** | `price.crypto` → `Chainlink` and `price.crypto.twap` → `Twap60`, both with `source = Polybolt`. Snapshots expand `payload.data[]`. The `op` frames map to their variants. `seq` gaps are detected by `PolyboltSeq` (snapshots consume a seq). |
| **Binance** | `*@bookTicker` → `CexQuote{Binance, asset, b, a}`. `*@trade` → `[]` (recorded raw only). |
| **Coinbase** | `ticker` with product `USDT-USD` → `UsdtUsd{price}`. Other products → `CexQuote{Coinbase, asset, best_bid, best_ask}`. `heartbeat`/`subscriptions` → `[]`. |

- [ ] **Step 1: Write the failing tests** in `tests/parsers.rs`. Load the fixtures with a helper `fn lines(path) -> Vec<(i64,String)>` reading `../../fixtures/<path>`.
  - `clob_fixture_parses_without_errors`: every line of `clob/live_window.jsonl` and `clob/after_window_end.jsonl` parses `Ok`. Counts across `live_window`: Book ≥ 2, PriceChange > 500, LastTrade ≥ 1, BestBidAsk ≥ 1.
  - `clob_snapshot_array_yields_two_books_with_best_last`: the first recv frame of `live_window.jsonl` is an array giving 2 `Book` events. Bids are ascending (`bids.last()` is the max price) and asks descending (`asks.last()` is the min).
  - `clob_market_resolved`: the `market_resolved.jsonl` line gives `MarketResolved{ winning_outcome: "Up", winning_asset_id: "4433802908…5816" (full string from fixture), condition_id: "0x1e5fd68c…2d82" (full) }`.
  - `clob_tick_size_change_parsed`: `after_window_end.jsonl` contains ≥ 2 `TickSize` with `new_tick == dec!(0.001)`.
  - `rtds_twap_at_boundary_matches_gamma`: across `rtds/boundary.jsonl`, there is a `Twap60` event for Btc with `ts_ms == 1793039400000` and `value == 80121.11048035804` (exact f64 equality).
  - `rtds_empty_frame_ok`: `parse_rtds("", 0) == Ok(vec![])`.
  - `polybolt_examples`: every `asyncapi_examples.jsonl` line parses. `priceCryptoTwap:live` gives `Twap60{Btc, value 64120.7, ts 1788973002000, source Polybolt}`, and the snapshot example gives 2 events. `authed` → `Authed`, and `error:auth_required` → `Error{code:"auth_required"}`.
  - `polybolt_seq_gap_detected`: `check("price.crypto",1)=None`, `check(…,2)=None`, `check(…,5)=Some((3,5))`.
  - `binance_book_ticker`: the fixture gives `CexQuote` events for all three assets with `bid < ask`.
  - `coinbase_ticker_and_usdt`: the fixture gives ≥ 1 `UsdtUsd` with a price between 0.98 and 1.02, and ≥ 1 `CexQuote{Coinbase, Btc}`.
  - `garbage_is_error_not_panic`: `parse_clob("{not json", 0).is_err()`, and the same for every other parser.
- [ ] **Step 2: Run them to make sure they fail.** Run `cargo test -p raki-feeds --test parsers`. Expected: FAIL.
- [ ] **Step 3: Implement the parsers** with serde structs that borrow `&str` where practical. Decimal fields use `rust_decimal::serde::str`.
- [ ] **Step 4: Run the tests to make sure they pass.** Run `cargo test -p raki-feeds --test parsers`. Expected: PASS.
- [ ] **Step 5: Commit.** `git add backend/crates/feeds/src/parse backend/crates/feeds/tests/parsers.rs && git commit -m "feat(feeds): pure parsers for clob, rtds, polybolt, binance, coinbase"`

---

### Task 4: WebSocket feed runners (`raki_feeds::{status, backoff, ws_conn, runners}`)

**Files:** `backend/crates/feeds/src/{status.rs, backoff.rs, ws_conn.rs, runners/*.rs}`, `backend/crates/feeds/tests/runners.rs`.

**Interfaces:**
- Consumes: the Task 2 `RecordTx`, `Record`, `RecordKind`; the Task 3 parsers.
- Produces:

```rust
pub struct FeedStatusRegistry;  // Arc-shared, interior Mutex<HashMap<Source, FeedState>>
impl FeedStatusRegistry {
    pub fn new(enabled: &[(Source, bool)], stale_ms: HashMap<Source, i64>) -> Arc<Self>;
    pub fn on_connected(&self, s: Source); pub fn on_disconnected(&self, s: Source, err: Option<String>);
    pub fn on_message(&self, s: Source, now_ms: i64);
    pub fn snapshot(&self, now_ms: i64) -> Vec<FeedStatusDto>;   // msgs_per_sec over trailing 10 s; stale = enabled && (last_msg older than stale_ms || !connected)
}
pub struct Backoff;  impl Backoff { pub fn new(initial: Duration, max: Duration) -> Self; pub fn next_delay(&mut self) -> Duration; pub fn reset(&mut self); }
    // initial 1s, x2, max 30s, full jitter in [d/2, d] using a cheap xorshift seeded from now_ns
pub struct FeedCtx { pub rec: RecordTx, pub events: mpsc::Sender<FeedEvent>, pub status: Arc<FeedStatusRegistry>,
                     pub reconnect: broadcast::Receiver<()>, pub shutdown: CancellationToken /* tokio-util */ }
pub async fn run_clob(ctx: FeedCtx, url: String, desired: watch::Receiver<BTreeSet<String>>);
pub async fn run_rtds_legacy(ctx: FeedCtx, url: String, assets: Vec<Asset>);
pub async fn run_polybolt(ctx: FeedCtx, url: String, creds: PolyCreds, assets: Vec<Asset>);
pub async fn run_binance(ctx: FeedCtx, base_url: String, assets: Vec<Asset>);
pub async fn run_coinbase(ctx: FeedCtx, url: String, assets: Vec<Asset>);
pub struct PolyCreds { pub api_key: SecretString, pub secret: SecretString, pub passphrase: SecretString }
impl PolyCreds { pub fn from_env() -> Option<PolyCreds>; }   // all three env vars present & non-empty
```

**Common runner contract.** Each runner loops until `shutdown`:
1. Connect.
2. Record `Open` (d = url with no secrets), call `status.on_connected`, and send `FeedEvent::FeedReset{source}`.
3. Send the subscribe/auth frames, recording each as `Send`. For PolyBolt, the `d` of the auth frame is redacted to `{"op":"auth","rid":"a1","auth":"<redacted>"}`.
4. Loop over `select!` of: next WS message, heartbeat tick, desired-token change (CLOB only), `reconnect` broadcast, and `shutdown`.
5. For every text message:
   - `rec.send(Record{k: Text, d: exact text})` **first**;
   - then `status.on_message`;
   - then parse and forward events with `events.try_send`. If full, log at warn level, rate-limited.
   - Parse errors are logged and counted, never fatal.
6. On close or error: record `Close` or `Error` (d = code/reason/err), then `Gap` (d = `{"source":..,"reason":..}`). Call `status.on_disconnected`, wait `backoff.next_delay()`, then reconnect.
   - The backoff resets after 60 s connected.
   - A clean server close 1000 on CLOB reconnects after 250 ms without increasing the backoff.
7. A `reconnect` broadcast (sleep/wake) drops the socket immediately and reconnects.

**Per-feed specifics:**

| Feed | Specifics |
|---|---|
| **CLOB** `wss://ws-subscriptions-clob.polymarket.com/ws/market` | Initial frame `{"assets_ids":[...all desired...],"type":"market","custom_feature_enabled":true}`. While connected, diff the desired set: `{"assets_ids":[added],"operation":"subscribe"}` and `{"assets_ids":[removed],"operation":"unsubscribe"}`. Text `PING` every 5 s; no `PONG` for 15 s → reconnect. If the desired set is empty, stay connected without subscribing. |
| **RTDS legacy** `wss://ws-live-data.polymarket.com` | One frame `{"action":"subscribe","subscriptions":[{"topic":"crypto_prices_twap_sixty","type":"*","filters":"{\"symbol\":\"btc/usd\"}"},{"topic":"crypto_prices_chainlink","type":"*","filters":"{\"symbol\":\"btc/usd\"}"}, …per asset]}`. Text `ping` every 5 s; no reply is expected. Stale data (no message for 15 s) → reconnect. |
| **PolyBolt** `wss://ws-live-v2.polymarket.com/ws` | Auth frame `{"op":"auth","rid":"a1","auth":{"apiKey","secret","passphrase"}}`. Wait for `authed` (a 10 s timeout or `error` → record `Error`, back off; on `auth_invalid` mark `last_error = "auth_invalid"` and back off at the 30 s max). Then one batch subscribe with `price.crypto` and `price.crypto.twap` (`window_seconds: 60`) per asset. App ping `{"op":"ping","rid":"p<n>"}` every 15 s. Close code 4008 → back off at the max and log at error level. `SeqGap` → record a `Gap`. |
| **Binance** | `<base>/stream?streams=btcusdt@bookTicker/btcusdt@trade/...`. Protocol pings are answered by tungstenite. Proactively reconnect every 23 h. |
| **Coinbase** `wss://ws-feed.exchange.coinbase.com` | Subscribe `{"type":"subscribe","product_ids":[assets…, "USDT-USD"],"channels":["ticker","heartbeat"]}` immediately after connecting. |

`ws_conn.rs`: `pub async fn connect(url: &str) -> Result<WsStream, WsError>`, using `connect_async_tls_with_config(url, None, true /*disable_nagle*/, None)`. The process-level rustls provider is installed by `main`; tests install it in a `#[ctor]`-free helper `ensure_crypto()` that is called at the start of each test.

- [ ] **Step 1: Write the failing tests** in `tests/runners.rs`. Use a local fake server (`tokio_tungstenite::accept_async` on `127.0.0.1:0`) that scripts its behaviour, `ws://` URLs, and paused-free real time with short timeouts (each test < 5 s). Backoff in tests is configurable through `Backoff::new(10ms, 50ms)`; give each `run_*` an internal `*_with` variant taking a `Backoff` and intervals.
  - `clob_sends_initial_subscribe_with_custom_features`: the server's first received text parses as JSON with `type=="market"`, `custom_feature_enabled==true`, and `assets_ids` equal to the desired set.
  - `clob_dynamic_subscribe_and_unsubscribe`: change desired from {a,b} to {b,c}. The server receives `operation:"subscribe"` with `["c"]` and `operation:"unsubscribe"` with `["a"]`.
  - `clob_reconnects_and_resubscribes_after_server_close`: the server sends one `book` frame, then closes with code 1000. The client reconnects (a second accept happens) and sends an initial subscribe containing the full desired set. The recorder receives, in order, `Open`, `Send`, `Text`, `Close`, `Gap`, `Open`.
  - `clob_sends_ping_and_accepts_pong`: with a 50 ms ping interval, the server receives `PING`, replies `PONG`, and no reconnect happens within 300 ms.
  - `raw_text_recorded_before_parse_even_if_garbage`: the server sends `{not json`. The recorder gets a `Text` record with exactly that `d`, and no event is emitted.
  - `polybolt_auth_redacted_in_record_and_subscribes_after_authed`: with dummy creds `k`/`s`/`p`, the server receives an auth frame containing `"apiKey":"k"`. The recorder's `Send` record for auth does not contain `"s"` as the secret value; assert `!d.contains("\"secret\"")`. After the server replies `{"op":"authed","rid":"a1"}`, it receives a subscribe frame with 2 subscriptions per asset.
  - `status_marks_stale_and_counts_reconnects`: after a disconnect, `snapshot()` shows `connected=false`, `stale=true`, `reconnects>=1`.
- [ ] **Step 2: Run them to make sure they fail.** Run `cargo test -p raki-feeds --test runners`. Expected: FAIL.
- [ ] **Step 3: Implement** `status.rs`, `backoff.rs`, `ws_conn.rs` and the five runners.
- [ ] **Step 4: Run the tests to make sure they pass.** Run `cargo test -p raki-feeds --test runners`. Expected: PASS.
- [ ] **Step 5: Run a manual live check.** Add an ignored test `live_feeds_smoke` (`#[ignore]`) that runs clob (desired set empty → no subscribe), rtds, binance and coinbase for 8 s against the real URLs. It asserts each recorded ≥ 1 `Text`; for clob, assert `Open` only. Run `cargo test -p raki-feeds --test runners -- --ignored live_feeds_smoke`. Expected: PASS.
- [ ] **Step 6: Commit.** `git commit -m "feat(feeds): resilient websocket runners with raw-first recording"`

---

### Task 5: REST clients (`raki_feeds::rest`)

**Files:** `backend/crates/feeds/src/rest/{mod.rs, gamma.rs, data_api.rs, geoblock.rs}`, `backend/crates/feeds/tests/rest.rs`.

**Interfaces:**
- Consumes: `raki_core::{GammaWindow, Outcome}`, `RecordTx` (each HTTP response body is recorded as `RecordKind::Http`; d = `{"url":..,"status":..,"body":..}`, except geoblock, whose body has `ip` removed).
- Produces:

```rust
pub struct PolyRest { /* reqwest::Client (timeout 10s), gamma_base, data_base, geoblock_url, rec: Option<RecordTx> */ }
pub struct RestUrls { pub gamma_base: String, pub data_base: String, pub geoblock_url: String }  // defaults: https://gamma-api.polymarket.com, https://data-api.polymarket.com, https://polymarket.com/api/geoblock
impl PolyRest {
    pub fn new(urls: RestUrls, rec: Option<RecordTx>) -> Self;
    pub async fn gamma_window(&self, slug: &str) -> Result<Option<GammaWindow>, RestError>; // GET {gamma}/events/slug/{slug}?cb={now_ms}; 404 → Ok(None)
    pub async fn resolution(&self, condition_id: &str) -> Result<Option<(Outcome, i64)>, RestError>; // GET {data}/v2/resolutions?condition={id}; data[] empty → None; payouts [up,down] → winner where payout>0; resolved_at RFC3339 → unix s
    pub async fn geoblock(&self) -> Result<GeoResult, RestError>;
}
pub struct GeoResult { pub blocked: bool, pub country: String, pub region: Option<String> }
pub fn parse_gamma_event(body: &str) -> Result<GammaWindow, RestError>;   // pure; used by tests
```

**`parse_gamma_event` mapping** (protocol notes, gamma §1):

| Field | Source |
|---|---|
| `start` | RFC3339 `.startTime`. Fall back to `markets[0].eventStartTime`. **Never use `.startDate`.** |
| `end` | `.endDate` |
| `condition_id` | `markets[0].conditionId` |
| `up_token`, `down_token` | `clobTokenIds` (JSON-encoded string) zipped with `outcomes` (JSON-encoded string), mapped by `"Up"`/`"Down"`. If index 0 is not `"Up"`, return `Err(Unexpected)`. |
| `price_to_beat`, `final_price` | `eventMetadata.priceToBeat` and `.finalPrice` (JSON numbers; the key may be absent) |
| `closed` | `.closed` |
| `twap_lookback_s` | `markets[0].cryptoMarketConfig.twapLookbackSeconds` |
| `fee_rate` | `markets[0].feeSchedule.rate` |
| `tick_size` | `markets[0].orderPriceMinTickSize` |

- [ ] **Step 1: Write the failing tests** (wiremock serving the fixture bodies):
  - `gamma_resolved_fixture`: `parse_gamma_event(btc_1793029800.json)` gives `start = 1793029800`, `end = 1793030100`, `price_to_beat == Some(80584.595188212)`, `final_price == Some(80399.2702888047)`, `closed`, `twap_lookback_s == Some(60)`, `fee_rate == Some(0.07)`. `up_token` equals the first id in `clobTokenIds`.
  - `gamma_live_fixture_has_no_metadata`: `btc_1793031000.json` gives `price_to_beat == None` and `closed == false`.
  - `gamma_eth_sol_fixtures_parse`: both parse, and their `slug` starts with `eth-`/`sol-`.
  - `gamma_404_is_none`: the mock returns 404 with `gamma/bad.json`, so `Ok(None)`.
  - `gamma_adds_cache_buster`: the wiremock matcher requires the query param `cb` to be present.
  - `resolution_resolved_and_pending`: `resolution_resolved.json` gives `Some((Down, 1793041…))`; compute the expected unix seconds from `resolved_at` in the fixture. `resolution_pending.json` gives `None`.
  - `geoblock_parses_and_record_strips_ip`: `blocked == false`, `country == "RS"`. The recorded `Http` record's `d` does not contain `203.0.113.7`.
- [ ] **Step 2: Run them to make sure they fail.** Run `cargo test -p raki-feeds --test rest`. Expected: FAIL.
- [ ] **Step 3: Implement** the REST client and parser as specified.
- [ ] **Step 4: Run the tests to make sure they pass.** Run `cargo test -p raki-feeds --test rest`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(feeds): gamma, data-api resolution and geoblock clients"`

---

### Task 6: Engine state machine (`raki-engine`)

**Files:** `backend/crates/engine/src/{book.rs, oracle.rs, cex.rs, windows.rs, series.rs, control.rs, engine.rs}`. Tests go in each module.

**Interfaces:**
- Consumes: `raki_core::*` (FeedEvent, dto types, WindowId).
- Produces:

```rust
// book.rs
#[derive(Default)] pub struct BookState { /* bids/asks: BTreeMap<Decimal, Decimal> */ }
impl BookState { pub fn apply_snapshot(&mut self, bids: &[Level], asks: &[Level]) -> bool /* true if differed from current state (resync) */;
                 pub fn apply_change(&mut self, side: Side, price: Decimal, size: Decimal);  // BUY→bids, SELL→asks; size 0 removes
                 pub fn best_bid(&self) -> Option<Decimal>; pub fn best_ask(&self) -> Option<Decimal>; }
// oracle.rs — keeps last 15 min of points per (asset, kind)
pub struct OracleState; impl OracleState { pub fn insert(&mut self, asset: Asset, kind: OracleKind, source: Source, ts_ms: i64, value: f64);
    pub fn latest(&self, asset: Asset, kind: OracleKind) -> Option<(i64, f64, Source)>;
    pub fn value_at(&self, asset: Asset, kind: OracleKind, ts_ms: i64) -> Option<(f64, Source)>; // exact ts match
    pub fn prune(&mut self, now_ms: i64); }
// If both RtdsLegacy and Polybolt deliver the same (asset, kind, ts), keep Polybolt.
// cex.rs
pub struct CexState; impl CexState { pub fn on_quote(&mut self, venue: Venue, asset: Asset, bid: Decimal, ask: Decimal, recv_ms: i64);
    pub fn on_usdt(&mut self, price: Decimal, recv_ms: i64);
    pub fn binance_mid_usd(&self, asset: Asset) -> Option<f64>;  // mid * usdt_usd (if usdt unknown → None)
    pub fn coinbase_mid(&self, asset: Asset) -> Option<f64>; }
// control.rs
pub struct Control { /* ControlState */ }
impl Control {
    pub fn new(now_ms: i64) -> Control;                 // Paused, StopKind::Startup, needs_ack=false
    pub fn state(&self) -> ControlState;
    pub fn pause(&mut self, reason: Option<String>, now_ms: i64) -> ControlState;     // → Paused, Manual
    pub fn kill(&mut self, reason: Option<String>, now_ms: i64) -> ControlState;      // → Killed, Manual
    pub fn resume(&mut self, safety: &SafetyChecks, acknowledge: bool, now_ms: i64) -> Result<ControlState, String>;
    pub fn on_safety(&mut self, safety: &SafetyChecks, now_ms: i64);  // Running && !all_ok → Paused{Safety, reason = reasons.join("; ")}; Startup && all_ok → Running (one-time auto-start)
}
// engine.rs
pub enum Request { FetchGamma { slug: String }, FetchResolution { slug: String, condition_id: String } }
pub enum EngineOutput { Series(Vec<SeriesRow>), Window(WindowRecord), Requests(Vec<Request>), DesiredTokens(BTreeSet<String>), BookResync { asset_id: String } }
pub struct EngineConfig { pub assets: Vec<Asset>, pub session_id: String }
pub struct Engine;
impl Engine {
    pub fn new(cfg: EngineConfig, now_ms: i64) -> Engine;
    pub fn handle(&mut self, ev: FeedEvent, now_ms: i64) -> Vec<EngineOutput>;
    pub fn tick(&mut self, now_ms: i64) -> Vec<EngineOutput>;    // called once per second
    pub fn live_windows(&self, now_ms: i64) -> Vec<LiveWindow>;  // current window per asset
    pub fn control(&mut self) -> &mut Control;
    pub fn book_resyncs(&self) -> u32;
}
```

**Resume rules (`Control::resume`):**
- If `!safety.all_ok`, return `Err(format!("safety checks failing: {}", reasons.join("; ")))`.
- Else if `needs_ack && !acknowledge`, return `Err("acknowledge required: <reason>")`.
- Else move to Running.
- `needs_ack` is true for `LossCap` and `ConfigHalt`; no M1 path sets those, but they are tested.
- Resume from Killed is allowed under the same rules.

**Window lifecycle** (driven by `tick` and `handle`). For each asset, the current and next window must be tracked.
- **Discovery:** an untracked window, or a tracked one without `condition_id`, emits `FetchGamma{slug}`, at most once per 5 s per slug.
- **`GammaFetched{Some(w)}`:** store the ids and tokens and Gamma's `price_to_beat` and `final_price`. Emit `Window` (upsert) and `DesiredTokens` if they changed.
- **`price_to_beat_ours`:** `oracle.value_at(asset, Twap60, start*1000)`, set once available. `twap_source` is the `Source` of that point.
- **`final_twap_ours`:** `value_at(asset, Twap60, end*1000)`. Once it exists, `outcome_ours = Up if final >= ptb_ours else Down` (a tie is Up).
- **After `end`:**
  - Emit `FetchResolution` every 5 s, starting at `end + 60 s`, until a winner is known.
  - Emit `FetchGamma` every 30 s, starting at `end + 120 s`, until `final_price_gamma` is known.
  - Either one stops after `end + 30 min` and sets the status `TimedOut`.
- **`MarketResolved`** (WebSocket) or **`Resolution{Some}`** sets `winner`, `resolved_at` (if known) and `resolution_seen_ms` (first time only). This is **latched**: later `None` results never clear it.
- **`twap_match`:** `Some(final_twap_ours == final_price_gamma)` when both are known (exact f64).
- **Status:**

  | Status | When |
  |---|---|
  | Upcoming | `now < start` |
  | Live | `start ≤ now < end` |
  | Ended | after `end`, no winner yet |
  | Resolved | winner known |
  | Complete | winner and `final_price_gamma` known |
  | TimedOut | 30 minutes passed after `end` |

  A window is dropped from memory 60 s after Complete or TimedOut.
- **Desired tokens:** the up and down tokens of every tracked window whose status is Upcoming, Live or Ended.
- **Series:** each `tick` emits one `SeriesRow` per Live window, with `ts = now_ms / 1000`:
  - the latest TWAP and Chainlink values;
  - the Binance mid in USD and the Coinbase mid;
  - best bid/ask for the up and down tokens.

  It increments `coverage_secs`.
- **Book handling:**
  - `Book` calls `apply_snapshot`; if it differed (true) **and** the asset already had state, increment `book_resyncs` and emit `BookResync`.
  - `PriceChange` entries apply to their own `asset_id`.
  - `FeedReset{ClobMarket}` clears all books.
- **Every `Window` output** carries the full current `WindowRecord`, with `updated_ms = now_ms`.

- [ ] **Step 1: Write the failing tests** (pure; no tokio):
  - **book.rs**
    - `book_replay_matches_checkpoints`: replay `fixtures/clob/live_window.jsonl` through `raki_feeds::parse::parse_clob` (dev-dependency on raki-feeds). Apply the snapshot, every `price_change`, then compare each later `Book` for that asset with `apply_snapshot`. At least 95% of the checkpoints return `false` (no difference), and the best bid/ask after the replay equal the last book's best.

      Note: the fixture contains two windows' tokens. Track books per `asset_id`.
    - `book_size_zero_removes_level`.
  - **oracle.rs**
    - `value_at_exact_ts_only`: insert ts 1000 and 2000. `value_at(1500)` is None and `value_at(2000)` is Some.
    - `polybolt_preferred_over_rtds_same_ts`.
  - **control.rs**
    - `startup_auto_runs_once_when_safe`: `on_safety(all_ok)` from Startup → Running. Then `on_safety(!all_ok)` → Paused/Safety. A later `on_safety(all_ok)` stays Paused, since the auto-start is one-time.
    - `resume_refused_when_unsafe` (error contains "safety checks failing").
    - `resume_requires_ack_for_loss_cap` (construct via a test-only `set_stopped(StopKind::LossCap, …)`).
    - `kill_then_resume_ok_when_safe`.
  - **engine.rs**: drive with synthetic events, using the start `1793031000`.
    - `discovery_requests_current_and_next`: on the first `tick` at `now = start*1000 + 1000`, `Requests` contains `FetchGamma` for the btc slugs of `start` and `start + 300`. A second tick within 5 s does not repeat them.
    - `gamma_fetched_sets_desired_tokens`.
    - `ours_ptb_and_final_from_twap_points_and_tie_is_up`: Twap60 points at `start*1000` (100.0) and `end*1000` (100.0) give `outcome_ours == Up`, and `twap_source == Some(RtdsLegacy)`.
    - `twap_match_true_when_equal_to_gamma_final`.
    - `resolution_latched`: a `Resolution{Some(Down)}` followed by a `Resolution{None}` keeps winner Down.
    - `market_resolved_ws_sets_winner_by_token`: the winning asset id equals `up_token`, so the winner is Up.
    - `resolution_requests_start_after_60s_every_5s`.
    - `gamma_final_polling_times_out`: with no final price by `end + 30 min`, the status is `TimedOut` and requests stop.
    - `series_rows_only_while_live_and_count_coverage`: 10 ticks during Live give 10 rows and `coverage_secs == 10`; ticks after `end` give no rows.
    - `feed_reset_clears_books`.
- [ ] **Step 2: Run them to make sure they fail.** Run `cargo test -p raki-engine`. Expected: FAIL.
- [ ] **Step 3: Implement** the engine modules as specified.
- [ ] **Step 4: Run the tests to make sure they pass.** Run `cargo test -p raki-engine`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(engine): books, oracle, window lifecycle, series, control"`

---

### Task 7: Postgres store (`raki-store`)

**Files:** `backend/crates/store/migrations/0001_init.sql`, `backend/crates/store/src/lib.rs`, `backend/crates/store/tests/store.rs`.

**Schema** (`0001_init.sql`):

```sql
create table sessions (id uuid primary key, started_ms bigint not null, ended_ms bigint, end_reason text, version text not null, host text not null);
create table windows (slug text primary key, asset text not null, start_ts bigint not null, end_ts bigint not null,
  event_id text, condition_id text, up_token text, down_token text,
  price_to_beat_ours double precision, final_twap_ours double precision, outcome_ours text,
  price_to_beat_gamma double precision, final_price_gamma double precision, twap_match boolean,
  winner text, resolved_at bigint, resolution_seen_ms bigint, twap_source text,
  coverage_secs integer not null default 0, status text not null, session_id uuid not null, updated_ms bigint not null);
create index windows_asset_start on windows (asset, start_ts desc);
create table window_series (slug text not null, ts bigint not null, twap double precision, chainlink double precision,
  binance_mid_usd double precision, coinbase_mid double precision, up_bid numeric, up_ask numeric, down_bid numeric, down_ask numeric,
  primary key (slug, ts));
create table feed_gaps (id bigserial primary key, session_id uuid not null, source text not null, at_ms bigint not null, reason text not null);
create table control_audit (id bigserial primary key, at_ms bigint not null, action text not null, ok boolean not null,
  from_mode text not null, to_mode text not null, reason text, error text);
```

**Interfaces — Produces:**

```rust
#[derive(Clone)] pub struct Store { pub pool: PgPool }
impl Store {
    pub async fn connect(url: &str) -> anyhow::Result<Store>;      // also runs sqlx::migrate!()
    pub async fn start_session(&self, id: Uuid, started_ms: i64, version: &str, host: &str) -> anyhow::Result<()>;
    pub async fn end_session(&self, id: Uuid, ended_ms: i64, reason: &str) -> anyhow::Result<()>;
    pub async fn upsert_window(&self, w: &WindowRecord) -> anyhow::Result<()>;   // ON CONFLICT(slug) DO UPDATE; keeps first session_id; coverage_secs = GREATEST(existing, excluded)
    pub async fn insert_series(&self, rows: &[SeriesRow]) -> anyhow::Result<()>;  // one multi-row INSERT, ON CONFLICT DO NOTHING
    pub async fn insert_gap(&self, session: Uuid, source: Source, at_ms: i64, reason: &str) -> anyhow::Result<()>;
    pub async fn insert_audit(&self, at_ms: i64, action: &str, ok: bool, from: TradingMode, to: TradingMode, reason: Option<&str>, error: Option<&str>) -> anyhow::Result<()>;
    pub async fn list_windows(&self, asset: Option<Asset>, limit: i64, before_start: Option<i64>) -> anyhow::Result<Vec<WindowRecord>>; // order by start_ts desc
    pub async fn get_window(&self, slug: &str) -> anyhow::Result<Option<WindowRecord>>;
    pub async fn get_series(&self, slug: &str) -> anyhow::Result<Vec<SeriesRow>>;   // order by ts
    pub async fn recording_stats(&self) -> anyhow::Result<RecordingStats>;
}
```

- **Coverage:** `coverage_secs` merges as `GREATEST(existing, excluded)`. The engine's per-window counter is monotonic within a session, and a restart mid-window loses at most that window's earlier count. This is accepted and documented in a code comment.
- **Enums:** stored as their serde snake_case/PascalCase strings.
- **Queries:** use `sqlx::query`/`query_as` with `&'static str` SQL and runtime binding, so no offline `.sqlx` is needed.
- **`recording_stats`:**

  | Field | Definition |
  |---|---|
  | `sessions` | count of sessions |
  | `recorded_secs` | `sum(coalesce(ended_ms, now_ms) - started_ms) / 1000` |
  | `windows_total` | count of windows |
  | `windows_with_twap` | windows with `final_twap_ours` not null |
  | `twap_match` / `twap_mismatch` | counts of `twap_match` true / false |
  | `twap_pending` | `final_twap_ours` not null and `twap_match` null |
  | `gate_target_windows` | 500 |

- [ ] **Step 1: Write the failing tests** with testcontainers `Postgres::default().with_tag("17-alpine")` and one container per test file (a `OnceCell`):
  - `migrations_apply`.
  - `upsert_window_roundtrip_and_update`: insert a window, update it with a winner, then `get_window` reflects it, keeps `session_id` from the first insert, and keeps the max `coverage_secs`.
  - `series_insert_idempotent`: inserting the same 3 rows twice leaves 3 rows, ordered by ts. Decimal values round-trip exactly (`dec!(0.995)`).
  - `list_windows_filters_and_orders`.
  - `recording_stats_counts`: 3 windows, one match, one mismatch and one pending; plus 2 sessions, one ended after 60 s. `recorded_secs >= 60` and the counts are exact.
  - `audit_and_gap_insert`.
- [ ] **Step 2: Run them to make sure they fail.** Run `cargo test -p raki-store`. Expected: FAIL.
- [ ] **Step 3: Implement** the migration and `Store`.
- [ ] **Step 4: Run the tests to make sure they pass.** Run `cargo test -p raki-store`. Expected: PASS (Docker must be running).
- [ ] **Step 5: Commit.** `git commit -m "feat(store): postgres schema and store"`

---

### Task 8: Health monitors (`traderd::health`)

**Files:** `backend/crates/traderd/src/health/{mod.rs, clock.rs, sleep.rs, geoblock.rs}`. Tests go in each module.

**Interfaces:**
- Consumes: `raki_feeds::rest::PolyRest::geoblock`, `raki_core::dto::*`.
- Produces:

```rust
pub struct HealthState { /* Mutex-protected ClockStatus, GeoStatus, sleep_events: u32, last_sleep_ms */ }
impl HealthState { pub fn new(max_offset_ms: f64) -> Arc<Self>; pub fn clock(&self) -> ClockStatus; pub fn geo(&self) -> GeoStatus; pub fn sleep_events(&self) -> u32; }
pub fn median_offset_ms(samples: &[f64]) -> Option<f64>;
pub async fn run_clock_monitor(state: Arc<HealthState>, servers: Vec<String>, interval: Duration, shutdown: CancellationToken);
    // rsntp AsyncSntpClient per server (2 s timeout each); offset_ms = rsntp clock_offset in ms as-is (positive = local clock behind); ok = |median| <= max
pub struct SleepDetector { /* last (Instant, monotonic_ns) */ }
impl SleepDetector { pub fn new() -> Self; pub fn check(&mut self, instant_elapsed: Duration, monotonic_elapsed: Duration) -> Option<Duration>; } // returns slept duration if monotonic - instant > 2 s
pub async fn run_sleep_monitor(state: Arc<HealthState>, rec: RecordTx, reconnect: broadcast::Sender<()>, shutdown: CancellationToken);
    // every 1 s read Instant::now() and libc::clock_gettime(CLOCK_MONOTONIC); on detection: sleep_events += 1, record Record{s: System, k: Sleep, d: {"slept_ms":..}}, reconnect.send(())
pub async fn run_geoblock_monitor(state: Arc<HealthState>, rest: PolyRest, interval: Duration, shutdown: CancellationToken);
pub fn safety(feeds: &[FeedStatusDto], clock: &ClockStatus, geo: &GeoStatus, rec: &RecorderStatus, rec_dropped_last_min: u64) -> SafetyChecks;
```

**`safety` rules:**
- `feeds_fresh`: every **enabled** feed is not stale.
- `clock_ok`: `clock.ok`. An unknown offset counts as **not ok**, with the reason `"clock offset unknown"`.
- `geoblock_ok`: `geo.blocked == Some(false)`. Unknown counts as not ok.
- `recorder_ok`: `rec_dropped_last_min == 0`, and `last_flush_ms` within 5 s.
- Each failing check adds a human-readable reason (e.g. `"feed stale: coinbase"`, `"clock offset 63.1 ms > 50 ms"`, `"geoblocked: US"`).

- [ ] **Step 1: Write the failing tests:**
  - `median_offset`: `[1.0, 50.0, 3.0]` gives 3.0, and empty gives None.
  - `sleep_detector_flags_drift`: `check(1s, 1s)` gives None, and `check(1s, 5s)` gives `Some(4s)`.
  - `safety_reasons`: build inputs with a stale coinbase, offset 63.1 (max 50) and geo unknown. `all_ok == false`, and the reasons contain `"feed stale: coinbase"`, `"clock offset 63.1 ms > 50 ms"` and `"geoblock status unknown"`.
  - `safety_all_ok`.
  - `#[ignore] live_clock_offset`: querying the real servers gives `Some(offset)` with `|offset| < 5000`.
- [ ] **Step 2: Run them to make sure they fail.** Run `cargo test -p traderd health`. Expected: FAIL.
- [ ] **Step 3: Implement** the monitors and `safety`.
- [ ] **Step 4: Run the tests to make sure they pass.** Run `cargo test -p traderd health && cargo test -p traderd -- --ignored live_clock_offset`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(traderd): clock, sleep and geoblock health monitors"`

---

### Task 9: HTTP and WebSocket API (`raki-api`)

**Files:** `backend/crates/api/src/{lib.rs, routes.rs, ws.rs}`, `backend/crates/api/tests/api.rs`.

**Interfaces:**
- Consumes: the Task 7 `Store`, `raki_core::dto::*`.
- Produces:

```rust
pub enum ControlCommand { Pause { reason: Option<String> }, Resume { acknowledge: bool }, Kill { reason: Option<String> } }
pub struct ControlMsg { pub cmd: ControlCommand, pub reply: oneshot::Sender<ControlResponse> }
#[derive(Clone)] pub struct AppState { pub store: Store, pub live: watch::Receiver<LiveSnapshot>, pub control: mpsc::Sender<ControlMsg> }
pub fn router(state: AppState, cors_origin: &str) -> Router;
```

**Routes** (all JSON; errors are `{"error": "..."}` with the right status):

| Route | Returns |
|---|---|
| `GET /api/live` | `LiveSnapshot` (the current watch value) |
| `GET /api/health` | `HealthSnapshot` |
| `GET /api/stats` | `RecordingStats` |
| `GET /api/windows?asset=btc&limit=50&before=<start_ts>` | `Vec<WindowRecord>`. `limit` is clamped to 1..=500 (default 50); an invalid asset gives 400. |
| `GET /api/windows/{slug}` | `WindowRecord`, or 404 |
| `GET /api/windows/{slug}/series` | `Vec<SeriesRow>` (an empty vec if unknown) |
| `POST /api/control/pause`, `/resume`, `/kill` | body `ControlRequest` (optional). Sends a `ControlMsg` and waits ≤ 2 s for the reply → `ControlResponse` (200 even when `ok == false`; 503 if no reply) |
| `GET /api/ws` | Upgrades. Sends the current `LiveSnapshot` immediately, then every time the watch changes; serialized JSON text frames. Client messages are ignored and the socket closes on client close. |

CORS: `allow_origin(cors_origin)`, methods GET and POST, header `content-type`.

- [ ] **Step 1: Write the failing tests** (testcontainers DB as in Task 7; `tower::ServiceExt::oneshot` for HTTP; a real bound listener plus a tokio-tungstenite client for WS):
  - `live_returns_snapshot`.
  - `windows_list_and_get_and_404`.
  - `windows_bad_asset_400`.
  - `series_endpoint_returns_rows_in_order`.
  - `stats_endpoint`.
  - `control_resume_roundtrip`: a test task answers `ControlMsg` with `ok:false, error:"safety checks failing: x"`. The response JSON has `ok == false` and that error.
  - `control_timeout_503`: nobody answers, so 503.
  - `ws_pushes_initial_and_updates`: the first frame parses as a `LiveSnapshot`; after `watch.send(updated)`, a second frame arrives with the new `generated_ms`.
  - `cors_allows_dashboard_origin`: preflight with `Origin: http://localhost:3000` gets `access-control-allow-origin` set to that origin.
- [ ] **Step 2: Run them to make sure they fail.** Run `cargo test -p raki-api`. Expected: FAIL.
- [ ] **Step 3: Implement** the router, handlers and WS push.
- [ ] **Step 4: Run the tests to make sure they pass.** Run `cargo test -p raki-api`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(api): read API, control endpoints, live websocket"`

---

### Task 10: `traderd` wiring, config, shutdown, live smoke run

**Files:** `backend/crates/traderd/src/{main.rs, config.rs, shell.rs, shutdown.rs}`, `backend/crates/traderd/src/bin/rawcat.rs`, `config/traderd.toml`, and a `README.md` section "Run M1 locally".

**Interfaces:**
- Consumes: everything above.
- Produces:
  - **CLI:** `traderd --config <path>` (clap derive). Exit code 0 on Ctrl-C.
  - **`config.rs`:** `pub struct Config` (serde, mirrors the TOML in Task 1) and `pub fn load(path: &Path) -> anyhow::Result<Config>`. Env overrides: `RAKI_DATABASE_URL` and `RAKI_API_BIND`, if set.
  - **`shell.rs`:** `pub async fn run_engine_loop(engine: Engine, events_rx, control_rx, store: Store, rest: PolyRest, desired_tx: watch::Sender<BTreeSet<String>>, live_tx: watch::Sender<LiveSnapshot>, health inputs…, shutdown)`. It runs a single task: `select!` over feed events, control messages and a 1 s interval.
    - It executes `EngineOutput`s:
      - `Series` → batched `insert_series`, flushed each tick;
      - `Window` → `upsert_window`;
      - `Requests` → spawned REST calls whose results come back as `FeedEvent::GammaFetched`/`GammaFailed`/`Resolution` on the same events channel;
      - `DesiredTokens` → `desired_tx.send`;
      - `BookResync` → a log line and `insert_gap`.
    - Each tick it computes `safety(...)`, calls `engine.control().on_safety(...)`, and publishes a `LiveSnapshot` to `live_tx`.
    - Control messages call `pause`/`resume`/`kill`, write `insert_audit`, and reply.
    - Store errors are logged at error level and the loop continues; they never crash it.

**`main` order:**
1. Install the rustls provider.
2. Set up tracing (env-filter, default `info`).
3. Load the config.
4. Create the data dir and run the store `connect` + migrate.
5. Create `session_id = Uuid::now_v7()` and call `start_session(host = hostname, version = env!("CARGO_PKG_VERSION"))`.
6. `spawn_recorder(data_dir, session, 200_000)`, then record `SessionStart` (d = `{"session":..,"version":..,"config":<config without secrets>}`).
7. Set up `FeedStatusRegistry` with the enabled flags. PolyBolt is enabled iff the config isn't `false` and `PolyCreds::from_env()` is Some. Log once whether PolyBolt is enabled; never log the values.
8. Create the channels: events mpsc 100_000, control mpsc 64, broadcast reconnect, watch desired, watch live.
9. Spawn the feed runners, the health monitors, the engine loop and the axum server.
10. On Ctrl-C/SIGTERM:
    - cancel the token;
    - record `SessionEnd`;
    - `end_session(reason="shutdown")`;
    - drop the record senders and await the recorder handle (finish);
    - exit within 5 s.

Feed gaps from runners are also written to `feed_gaps`: the runners' `Gap` records are mirrored by an `FeedEvent::FeedReset`. The shell inserts a gap row for every `FeedReset` after the first per source.

- [ ] **Step 1: Write the failing test** `config_loads_repo_config`: `load("../../../config/traderd.toml")` succeeds, `assets == [Btc, Eth, Sol]`, and `health.clock_max_offset_ms == 50.0`. Add `env_override_database_url`.
- [ ] **Step 2: Run it to make sure it fails.** Run `cargo test -p traderd config`. Expected: FAIL.
- [ ] **Step 3: Implement** `config.rs`, `shell.rs`, `shutdown.rs` and `main.rs`.
- [ ] **Step 4: Run the whole workspace.** Run `cd backend && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Run a live smoke test** of at least 7 minutes, so it spans one full window end plus resolution. Run `make db-up`, start `traderd` in the background with logs going to `data/smoke.log`, and wait about 7 minutes. Then check:
  - `curl -s 127.0.0.1:8787/api/health`: every enabled feed has `connected:true`, and `recorder.records > 1000`.
  - `curl -s '127.0.0.1:8787/api/windows?asset=btc&limit=5'`: at least one window with `price_to_beat_ours` not null and `coverage_secs > 60`.
  - The ended window shows a `winner` within about 3 minutes of its end.
  - `final_twap_ours` is not null.
  - Stop with SIGINT. The process exits 0 within 5 s, `sessions.ended_ms` is set, and the newest raw file decodes with `read_file` via a small `cargo run -p traderd --bin rawcat -- <file>` helper. `rawcat` prints per-source record counts and `truncated=false`.
  - If `twap_match` is still pending because Gamma is 4–6 minutes late, wait and re-query; it must become `true`. Document the observed values in the commit message.
- [ ] **Step 6: Commit.** `git commit -m "feat(traderd): wire recorder, feeds, engine, store, api; live smoke verified"`

## Self-review notes

- **Spec coverage.** Each M1 requirement in CLAUDE.md §5 maps to a task:

  | Requirement | Task |
  |---|---|
  | Record CLOB, PolyBolt/TWAP, Binance, Coinbase, config and resolutions | Tasks 3–5, 10 |
  | Session boundaries | Tasks 2, 7, 10 |
  | Gaps explained | Tasks 4, 8, 10 |
  | `finalPrice` check | Task 6 (`twap_match`) and Task 7 (stats) |
  | Dashboard data | Task 9 |
  | Pause/resume/kill with safety | Tasks 6, 9, 10 |
  | Sleep handling | Task 8 |
  | Geoblock/clock | Task 8 |
- **PolyBolt without credentials.** Legacy RTDS supplies the TWAP until credentials exist; `twap_source` records which source was used.
- **Out of scope for M1:** strategy, ledger, orders, alerts (Telegram) and parameter editing. These are deferred by the spec, and alerts move to the M3 plan.

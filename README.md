# raki-trader

A recorder, and later a trading bot, for Polymarket's 5-minute crypto "Up or Down" markets. See
`CLAUDE.md` for the full picture.

## Run M1 locally

M1 records market data and serves the dashboard API. It places no orders.

Prerequisites: Rust (`source ~/.cargo/env`), Docker, and on macOS `caffeinate`, which comes with
the OS.

```sh
make db-up          # Postgres 17 on 127.0.0.1:5433 (local dev credentials raki/raki)
make backend-run    # traderd --config config/traderd.toml, kept awake with caffeinate
```

Run both from the repo root. `data_dir = "data"` in `config/traderd.toml` resolves against the
working directory.

- **Recording.** Raw frames go to `data/raw/YYYY-MM-DD/HH-<session>-<n>.ndjson.zst`. Each start
  opens a new session, which is stored in Postgres (`sessions`) and in the raw stream
  (`session_start`/`session_end`). Recording runs only while the process runs.
- **API** on `127.0.0.1:8787`:
  - `GET /api/health` returns feeds, clock, geoblock, recorder and safety.
  - `GET /api/live` returns the same plus the current windows and the control state.
  - `GET /api/windows?asset=btc&limit=5` and `GET /api/stats` return recorded windows and gate
    progress.
  - `/api/ws` pushes live updates.
- **Stopping.** Press Ctrl-C (or send SIGTERM). It exits 0 within 5 s, closes the session row and
  finishes the raw file.
- **Inspecting a raw file:**
  `cargo run --release --manifest-path backend/Cargo.toml -p traderd --bin rawcat -- data/raw/<day>/<file>.ndjson.zst`
  prints per-source record counts and `truncated=true|false`.
- **PolyBolt** (Chainlink price and TWAP) turns on only when `POLY_API_KEY`, `POLY_API_SECRET`
  and `POLY_API_PASSPHRASE` are all set in the environment. Never put them in the config file.
  Until then, the legacy RTDS feed supplies the TWAP, and each window's `twap_source` says which
  feed it came from.
- **Overrides.** `RAKI_DATABASE_URL` and `RAKI_API_BIND` override the config file. `RUST_LOG`
  sets the log level (default `info`).
- **Clock.** Trading safety requires an NTP offset of 50 ms or less. If `/api/health` reports a
  larger offset, resync the clock (macOS: `sudo sntp -sS time.apple.com`).
- **Disk.** All three assets' CLOB books are recorded. Expect a few hundred MB of compressed raw
  data per hour.

`make db-down` stops Postgres. The data volume `raki-pg` is kept.

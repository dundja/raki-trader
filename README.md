# raki-trader

A recorder, and later a trading bot, for Polymarket's 5-minute crypto "Up or Down" markets. See
`CLAUDE.md` for the full picture.

## Run M1 locally

M1 records market data and shows it on a local dashboard. It places no orders.

### Prerequisites

- **Docker**, for Postgres.
- **Rust** (stable) via rustup (`source ~/.cargo/env`).
- **Node 22.12+ and pnpm**, for the dashboard. Run `cd frontend && pnpm install` once.
- On macOS, `caffeinate`, which comes with the OS and keeps the machine awake while recording.

### Start

```sh
make dev            # Postgres, then traderd and the dashboard dev server side by side
```

`make dev` runs these, which also work one at a time in separate terminals:

```sh
make db-up          # Postgres 17 on 127.0.0.1:5433 (local dev credentials raki/raki)
make backend-run    # traderd --config config/traderd.toml, kept awake with caffeinate
make frontend-dev   # dashboard dev server (vite) on port 3000
```

Run them from the repo root. `data_dir = "data"` in `config/traderd.toml` resolves against the
working directory.

### URLs

- **Dashboard:** <http://localhost:3000>. Use `localhost`, not `127.0.0.1`: the API allows only
  the `http://localhost:3000` origin (`cors_origin` in `config/traderd.toml`).
  - **Live:** the open BTC, ETH and SOL windows.
  - **Health:** feeds, clock, geoblock, recorder, safety checks, M1 gate progress and the
    Pause / Resume / Kill controls.
  - **Windows:** recorded windows and a per-window chart.
- **API** on <http://127.0.0.1:8787>:
  - `GET /api/health` returns feeds, clock, geoblock, recorder and safety.
  - `GET /api/live` returns the same plus the current windows and the control state.
  - `GET /api/windows?asset=btc&limit=5` and `GET /api/stats` return recorded windows and gate
    progress.
  - `/api/ws` pushes live updates.
- The dashboard reads the API base from `VITE_API_URL` (default `http://127.0.0.1:8787`); see
  `frontend/.env.example`.

### Stop

Press Ctrl-C in the `make dev` terminal. It stops traderd and the dev server; the database keeps
running. traderd exits 0 within 5 s on Ctrl-C or SIGTERM, closes the session row and finishes the
raw file. `make db-down` stops Postgres. The data volume `raki-pg` is kept.

### Where data lives

- **Raw frames** go to `data/raw/YYYY-MM-DD/HH-<session>-<n>.ndjson.zst` (gitignored). They are the
  source of truth for replay and are never deleted. Expect a few hundred MB of compressed data per
  hour, since all three assets' CLOB books are recorded.
- **Postgres** (volume `raki-pg`) holds sessions, windows (with the TWAP-vs-Gamma comparison),
  the per-window 1 s series, feed gaps and the audit log of control actions.
- Each start opens a new session, which is stored in Postgres (`sessions`) and in the raw stream
  (`session_start`/`session_end`). Recording runs only while the process runs.

### Inspecting a raw file

```sh
cargo run --release --manifest-path backend/Cargo.toml -p traderd --bin rawcat -- data/raw/<day>/<file>.ndjson.zst
```

It prints per-source record counts and `truncated=true|false`.

### Configuration and safety

- **PolyBolt** (Chainlink price and TWAP) turns on only when `POLY_API_KEY`, `POLY_API_SECRET`
  and `POLY_API_PASSPHRASE` are all set in the environment. Never put them in the config file.
  Until then, the legacy RTDS feed supplies the TWAP, and each window's `twap_source` says which
  feed it came from.
- **Overrides.** `RAKI_DATABASE_URL` and `RAKI_API_BIND` override the config file. `RUST_LOG`
  sets the log level (default `info`).
- **Clock.** Trading safety requires an NTP offset of 50 ms or less. Above that the bot pauses
  itself and refuses Resume with "safety checks failing: clock offset …". Resync the clock
  (macOS: `sudo sntp -sS time.apple.com`).

### Tests

```sh
make test                        # backend (cargo test) and dashboard unit tests (vitest)
cd frontend && pnpm e2e          # Playwright smoke against the running stack (make dev first)
```

The smoke test drives the real backend: it pauses trading and tries to resume it. It never
clicks Kill. Before the first run, install the browser with
`cd frontend && pnpm exec playwright install chromium`. Screenshots go to
`frontend/e2e/screenshots/` (gitignored).

# raki-trader

An automated trading bot for Polymarket's **5-minute crypto "Up or Down" markets**. It has a Rust backend that holds all the trading logic and a React/TypeScript (TanStack Start) dashboard for watching what the bot does and how it performs.

> **Status (2026-10-26):** pre-code. Everything (Rust server and dashboard) runs on the user's **local machine** for now. The bot records and trades **only while the app is running**; stopping the app stops everything. Cloud deployment comes later (§8). This file is the project's north star. Detailed specs per milestone live in `docs/specs/`. The research this is based on is in `docs/research/2026-10-26-polymarket-5m-crypto-research.md`. Read its "Corrections" section first.

---

## 1. What we're building and why

Every 5 minutes Polymarket opens a market such as "Will BTC be up or down at the end of this 5-minute window?". Shares of the winning side pay $1 and shares of the losing side pay $0. The price of a share is the crowd's implied probability.

**The idea:** late in a window, the outcome is often nearly decided. If the bot reads the live price feeds (Chainlink, Binance, Coinbase) better than the order book does, it can buy the likely winner at a price below its true probability. Repeating that across many windows adds up to profit.

**The hard truth we design around:**
- Volume does not create an edge. It only multiplies whatever edge each bet already has, positive or negative.
- At 92¢ a win earns 8¢ and a loss costs 92¢. One loss wipes out about 11–12 wins.
- Most public evidence says this exact strategy is crowded and roughly break-even or losing.
- So this project is built as **a measurement machine first and a trading bot second**:
  - Record everything.
  - Prove an edge on data, with numbers agreed in advance.
  - Only then risk money, and only small amounts at first.

### Success looks like
- The bot makes money over thousands of trades. The result holds up statistically after fees and spread.
- For every trade we can see why it was taken, what the model believed, what we paid, and how it settled.
- The bot never loses more than the configured limits, even when something breaks.

### Non-goals (for now)
- Other Polymarket markets (sports, politics, hourly or daily crypto). The hourly and daily crypto markets settle differently: they use Binance candles, not Chainlink TWAP.
- Market making as a business, or providing liquidity for rebates as the main strategy. Maker entries may be *tested* as a variant.
- Multi-user support or offering this as a product.
- Speed races against co-located bots. We cannot win those and we don't try.
- Doing anything with crypto beyond what Polymarket requires. The user funds the account by card and wants money out to their bank or card.

### Anti-goals (failure even if the code "works")
- **Paper results that look great but are unrealistic** (fills at the displayed price, ignoring fees, spread, queue position and the 150 ms delay). This is the most likely way to fool ourselves.
- Scaling up stake size or volume before the go/no-go gate passes.
- Losing funds because of key leakage, a bug in live order code, or retrying an order that had already succeeded.
- An account freeze caused by hosting or VPN choices.
- Hard-coded platform parameters that silently go stale when Polymarket changes something. It changed fees, delays, the TWAP window, the API version and the price feeds several times in 2026.

---

## 2. How the markets actually work (verified 2026-10-26, re-verify at runtime)

| Topic | Fact | Design consequence |
|---|---|---|
| **Settlement** | A **Chainlink 60-second TWAP** stream, e.g. `btc-usd-twap-60s-streams`. It is *not* spot Binance and *not* a single price snapshot. Since 2026-08-14 the 5m lookback is 60 s (`cryptoMarketConfig.twapLookbackSeconds`). | The model must forecast the **remaining part of the 60 s average**. Binance and Coinbase are leading signals, not ground truth (Binance traded about 0.2–0.3% away from Chainlink). |
| **Price to beat** | Equal to the previous window's final TWAP value. | `priceToBeat` can be `null` for a while if the previous window hasn't resolved yet. Take it from our own recorded TWAP at the window boundary and reconcile with Gamma later. |
| **Rule** | Up if final TWAP **≥** price to beat, otherwise Down. **A tie resolves Up.** | Implement and test the `>=` exactly. |
| **What's fixed late in the window** | At T-30s about half of the averaging window has already been observed; at T-45s about a quarter. | The core of the model: observed part plus a volatility forecast for the rest. |
| **Windows** | Unix-aligned to multiples of 300 s. Slug: `{asset}-updown-5m-{unix_start}`. Created about 24 h ahead. Fetch with `GET gamma-api.polymarket.com/events/slug/<slug>`. | Compute slugs from the clock. Don't list by series, because that returns stale events. |
| **Assets** | BTC, ETH, SOL, XRP, DOGE, HYPE, BNB, ZEC. | Liquidity: BTC ≈ $12M/day, ETH and SOL ≈ $0.2–0.3M/day, the others are tiny. **BTC first.** |
| **Trading after the end** | Markets keep accepting orders after the window ends, until resolution. | Never leave resting orders across the window end. Faster players will pick them off. |
| **Resolution** | Automatic (no UMA). Median about 26 s, p90 about 85 s, with a tail of more than 2–4 min. Gamma `closedTime` is unreliable. | Measure resolution ourselves from CLOB flags and on-chain events. |
| **Redemption** | Winnings must be **redeemed** (gasless, through the Relayer; needs a Relayer API key). | We need an automated redeem loop. Capital is locked until it runs. |
| **Fees** | Taker fee = `shares × 0.07 × p × (1−p)`. Makers pay nothing and get a share of fees as a rebate. | Break-even win rate ≈ `p + 0.07·p·(1−p)`: about 92.5% at 0.92. **The spread costs more than the fee:** crossing 1¢ moves break-even to about 93.5%. |
| **Taker delay** | Marketable orders on crypto markets are held **150 ms** (since 2026-09-04) and cannot be cancelled while held. | Speed is not an edge. Simulate the delay in paper trading. |
| **Order rules** | Minimum 5 shares. Tick 0.01, switching to **0.001 near extreme prices** (read the tick from the CLOB, not Gamma). Order types GTC/GTD/FOK/FAK, post-only. Rate limit about 40 orders/s per signer. | Read `feeSchedule`, tick, min size and `itode` per market at runtime. |
| **Late-window liquidity** | In most sampled windows the favourite was already bid at 0.99 with **no asks** 20–75 s before the close. Entries at 92–97¢ appear mostly in *contested* windows. | A large share of windows will have nothing to buy. Contested windows are where reversals happen. Measure this before trusting the strategy. |

**APIs we use:**
- **CLOB V2** REST and WebSocket (market channel for books, user channel for fills).
- **Gamma** for discovering markets.
- **PolyBolt** (`wss://ws-live-v2.polymarket.com/ws`, topics `price.crypto` and `price.crypto.twap`) for Chainlink prices and the TWAP. It **needs CLOB API credentials**, so paper mode needs an account too.
- Legacy RTDS price topics are deprecated. Don't build on them.
- Binance and Coinbase public WebSockets as leading signals.
- The official Rust SDK is `polymarket_client_sdk_v2` (repo `Polymarket/rs-clob-client-v2`, pre-1.0). It has no PolyBolt client and has known WebSocket resubscribe bugs, so expect to hand-roll WebSocket clients.

---

## 3. Strategy v0: the first hypothesis to test

This is the user's starting rule. **Every number is a tunable parameter.** It is treated as a hypothesis to measure, not as a known winner.

**Enter when all of the following hold:**
1. Time left in the window is between `entry_window_end_s` and `entry_window_start_s` (default **30–45 s**).
2. The market prices one side at ≥ `min_market_prob` (default **0.92**). The price we would actually pay (best ask, walking the book) is ≤ `max_entry_price`.
3. The bot's own probability for that side, from the TWAP-aware model, beats the all-in cost by at least `min_edge`: `model_prob − (fill_price + fee_per_share) ≥ min_edge`.
4. All safety checks pass: feeds fresh, clock in sync, price to beat known, market config as expected, risk limits not hit, not geoblocked.

**Scope:** trade **BTC 5-minute markets only**, with at most one position per window. The recorder still captures ETH and SOL so other assets can be studied later without waiting.

**Order:** a taker FAK/FOK at a limit price, with a fixed stake of **$5 per window** (the user's decision; see §6).

**Variants to compare on recorded data later:**
- Earlier entry (T-60 to T-120 s, where a TWAP model has more to add).
- Post-only maker bids at 0.93–0.97.
- A filter that skips contested windows.
- More assets.

Rebates are never counted in the base economics. Polymarket can change them at any time.

---

## 4. Architecture

```
            ┌──────────── Rust backend (single process to start) ────────────┐
Polymarket  │  feeds ──► event bus ──► recorder (raw, lossless, append-only)  │
CLOB WS ───►│    │            │                                               │
PolyBolt ──►│    │            ├──► strategy core (pure: events → intents)     │
Binance ───►│    │            │          │                                    │
Coinbase ──►│    │            │          ▼                                    │
Gamma REST─►│ discovery       │     risk gate ──► Executor trait               │
            │                 │                    ├─ ReplayExecutor (backtest)│
            │                 │                    ├─ PaperExecutor (sim fills)│
            │                 │                    └─ LiveExecutor (real CLOB) │
            │                 ├──► ledger / positions / redeem loop / PnL     │
            │                 └──► api (HTTP + WS push) ──────────────────────┼──► Dashboard
            └────────────────────────────────────────────────────────────────┘      (TanStack Start)
```

**Principles:**
- **One strategy core, three executors.**
  - Replay (from recorded data), paper (live data, simulated fills) and live (real orders) all run *the same* strategy code on the same event stream.
  - Only the executor differs.
  - Replay must be deterministic: same input, same output.
- **The recorder comes first and is never optional.**
  - Ticks we didn't record are gone forever. Chainlink's exact TWAP method is unpublished, so our own recordings are the only way to calibrate.
  - Every event is stored with both the source timestamp and the local receive timestamp (ns), plus sequence numbers where the source provides them.
- **The paper fill simulator must be realistic.** For each simulated order:
  - Fill against the book as it was at decision time + measured round trip + 150 ms taker delay, not the book at decision time.
  - Walk the book level by level, with FAK/FOK semantics.
  - Reject if the price moved past the limit.
  - Enforce the 5-share minimum and the real fee formula with correct rounding.
  - Maker orders join at the back of the queue.
  - Log "wanted to trade but couldn't fill". That rate is a key metric.
- **Live mode is opt-in and hard to turn on by accident.** It requires an explicit config value *and* an environment flag, and it refuses to start if any safety precondition fails.
- **Fail closed.** On stale data, a disconnect, clock drift, a config mismatch, a geoblock or a reconciliation mismatch: stop opening positions, cancel resting orders and alert.

**Backend stack (my call, revisit in specs):**
- **Async runtime:** tokio.
- **Networking:** tokio-tungstenite for WebSockets, reqwest for REST, axum for the dashboard API.
- **Money and prices:** `rust_decimal`. Never `f64` for money, prices or the ledger.
- **Secrets:** `secrecy` / `zeroize` types.
- **Polymarket SDK:** the official `polymarket_client_sdk_v2` for signing and REST.
- **Repo layout:** a Cargo workspace. Suggested crates, to be refined in the M1 spec: `core` (domain types), `feeds`, `discovery`, `recorder`, `strategy`, `exec`, `risk`, `ledger`, `api`, and a `traderd` binary.

**Storage (my call, revisit in the M1 spec):**
- Raw events go to append-only, hourly-rotated, zstd-compressed files. They are the source of truth for replay.
  - They are **kept indefinitely**; older files may move to cheaper cold storage, but nothing is deleted.
- Trades, orders, positions, the ledger, per-window outcomes and derived metrics go into **Postgres**. Only the Rust backend reads and writes Postgres.
- The recorder also writes a **downsampled per-window series** to Postgres (about 1 s resolution: TWAP, Chainlink, Binance, Coinbase, best bid/ask, model probability), so the dashboard can chart any window without reading raw files.
- If analytics outgrow Postgres, move them to ClickHouse or Parquet.
- Every order, trade, decision and metric carries a **`run_id`, `mode` (replay / paper / live) and `strategy_id`**. Paper and live results are never mixed in one number or one equity curve.

**Frontend:**
- React + TypeScript on **TanStack Start**, with TanStack Query for data.
- It talks **only to the Rust API** (REST for history, WebSocket for live push) and never connects to the database. This is my call: the backend owns the schema, and API types are generated from Rust.
- **Monitoring and control.** It is not exposed to the public internet; for now it runs locally, and later access is over Tailscale/WireGuard. It never places individual orders by hand; the strategy places orders. The controls are:
  - **Pause:** stop opening new positions. Existing positions, reconciliation and redemption keep running.
  - **Resume:** start opening positions again after a pause, a kill, a loss cap or a config halt.
    - It needs an explicit confirmation, which says why the bot stopped.
    - The backend refuses to resume while any hard safety check still fails: stale feeds, clock offset, geoblock or a reconciliation mismatch.
    - Resuming after a loss cap or a config-fingerprint halt also needs the user to acknowledge that reason.
  - **Kill:** pause, plus cancel every resting order.
  - **Later (not in M1):** adjust strategy parameters from the dashboard (entry window, `min_market_prob`, `max_entry_price`, `min_edge`, stake, loss caps) and cancel specific resting orders.
    - Parameter changes are validated against the backend's allowed ranges and versioned. Each run records the exact parameter version it used.
    - During M3, changing the frozen configuration restarts the go/no-go gate, and the dashboard warns before applying.
- Every control action and parameter change goes in an audit log.
- No login is needed while everything is local. Authentication is added before the dashboard is reachable from anywhere other than the local machine.
- **Scope by milestone:**
  - M1: live window plus system health.
  - M2: backtest, variant comparison and calibration reports, selected by `run_id`.
  - M3 onwards: paper and live views.
- Money-path and tax views (balances in RSD, deposits and withdrawals) are not part of the dashboard at first. The ledger exports CSV for the accountant.
- Views, to be refined in the frontend spec:
  - **Live window:** the TWAP vs price to beat, time left, book, model probability vs market price, and the bot's decision with reasons.
  - **Trades and PnL:** equity curve, per-trade drill-down, fees paid.
  - **Calibration:** predicted vs realized win rate per price bucket; fill-rate stats.
  - **System health:** feed freshness, clock offset, geoblock status, kill-switch state, unredeemed positions, config fingerprint.

**Alerts:** pushed to a phone (Telegram or Pushover, my call) for the kill switch, stale feeds, geoblock changes, reconciliation mismatches, unredeemed positions, clock offset and the daily PnL summary. This matters most once the bot runs unattended. When it moves to the cloud, add an external watchdog that alerts if the bot goes silent for more than 60 s.

---

## 5. Milestones and gates

| # | Milestone | Done when |
|---|---|---|
| **M0** | **Account and money path** (mostly the user, partly manual) | Polymarket account exists. Wallet type identified (Deposit Wallet sig type 3 vs legacy proxy sig type 1). CLOB API credentials derived. **$20 card deposit and $20 withdrawal to the user's bank or card tested**, with fees and KYC steps written down. Geoblock check from the local machine returns `blocked:false`. |
| **M1** | **Recorder + health dashboard** | Whenever the app runs, it records CLOB books and trades plus PolyBolt price and TWAP for BTC/ETH/SOL 5m, Binance, Coinbase, per-market config and resolutions. Gaps from the app being stopped are expected; each one is logged as a recording session boundary. **At least 500 BTC windows recorded with PolyBolt** (about 42 hours of running time), and every gap *inside* a session is explained (a logged disconnect, sleep or crash). Our final PolyBolt TWAP matches Gamma `finalPrice` for those windows (if it doesn't, stop and investigate). The dashboard shows live windows and system health. Recording runs in **every later milestone**; the archive keeps growing. |
| **M2** | **Replay, model, simulator, backtest** | Deterministic replay, the TWAP estimator, a realistic fill simulator, and backtests of strategy v0 and its variants, with calibration reports per price bucket. We also measure how often a fillable 0.92–0.97 ask actually exists at T-45 to T-30 s. **M2 gate:** see below. |
| **M3** | **Paper trading (live data, simulated fills)** | Runs one **frozen** configuration for at least 4 weeks, including volatile weekdays (FOMC, CPI, liquidation days), and longer if needed to reach the sample size. Must pass the **go/no-go gate** below. |
| **M4** | **Live at minimum size** | 1–2 weeks of real fills at the smallest size. Every real fill is compared with what the simulator predicted for the same moment. Redemption and reconciliation are proven. |
| **M5** | **Scale carefully** | Size increases in small steps, and only while live results stay within the paper confidence bands. |

**M1 can start before M0 finishes (my call).** Public streams (CLOB books and trades, Gamma, Binance, Coinbase) need no credentials, so recording starts with them right away. PolyBolt (Chainlink price and TWAP) joins as soon as M0 provides CLOB API credentials. The 500-window count and the `finalPrice` check start once PolyBolt is live.

**Data source: our own recordings only** (the user's decision; no bought history). How fast M2 and M3 can progress therefore depends directly on **how many hours the app runs**. The dashboard shows total recorded windows and hours, so progress toward each gate is visible.

**M2 gate (my call, set before looking at results):**
- Split the recorded data by time. The older part is used for building and tuning; the newest part (at least 1 week, untouched until the end) is held out.
- Pick **one** configuration using the tuning part only, then evaluate it once on the held-out part with the realistic simulator.
- **Go to M3** only if the held-out edge after fees and spread is positive, *and* there are enough fillable opportunities to make the M3 sample size reachable in a reasonable time.
- Otherwise: redesign (a new hypothesis, then back through M2 on fresh held-out data) or stop. Don't run M3 on a strategy the data already rejects.

**Go/no-go gate for M3 → M4 (starting rule, the user may adjust; set before looking at results):**
- **One frozen configuration.** Other variants may run alongside in shadow mode, but only the frozen one counts. Retuning it mid-run restarts the gate.
- **Sample size:** at least 4 weeks *and* enough independent windows for the confidence bound to settle. About 2,000 qualifying trades is the target, not a fixed rule.
  - If trades come in slower (few windows have a fillable 0.92–0.97 ask), M3 runs longer. It doesn't lower the bar.
  - Backtest trades never count toward the gate.
- **Edge:** the **lower 95% confidence bound** of edge per dollar, after fees and the spread actually paid, is above 0.
  - Count each window as one sample, because trades in the same window are correlated.
  - The point estimate should also clear a **minimum worthwhile edge**, default **1¢ per $1 staked** (my call). Smaller edges are eaten by card-in/card-out costs (about 5–9% per round trip) and by model risk.
- **Calibration:** predicted vs realized win rate reported per bucket (0.92–0.95, 0.95–0.97, 0.97–0.99). No bucket can be significantly over-confident.
- **Drawdown:** max drawdown stays within the risk limits.
- **Platform changes mid-run:** if Polymarket changes fees, the taker delay, the TWAP lookback or the tick rules, the bot halts (config fingerprint). Trades from before the change are kept and reported separately. The gate clock restarts for the new setup, unless the M2 replay shows the change doesn't affect the strategy.
- If the gate fails: change the strategy or stop. **Never fix a failing gate with more volume.**

---

## 6. Risk management (defaults are my call, all configurable)

- **Stake per trade:** a fixed **$5 per BTC window** (the user's decision), which is about 5.4 shares at 92¢, just above the 5-share minimum. A win earns about $0.40; a loss costs about $4.60. The stake is a config value; changing it is a deliberate decision after the gate, never automatic. No Kelly sizing while the edge is unproven.
- **Per-window exposure:** at most one position per window. If more assets are traded later, all assets in the same 5-minute window count as **one** correlated bet with a combined cap.
- **Loss caps (my call, in dollars):** daily **$25** (about 5 losses), weekly **$75**. Hitting a cap pauses trading until the user resumes it from the dashboard and acknowledges the reason. Revisit these once the starting balance for live trading is known.
- **Bankroll for paper drawdown checks:** a config value. Paper mode uses an assumed starting balance; the user sets the real one at M4.
- **Only free balance can be traded.** We track three balances: free, in open positions, and resolved but not yet redeemed.
- **Kill-switch triggers:**
  - N losses in a row.
  - Realized win rate in a bucket drifting X standard deviations below the model.
  - A stale feed: the TWAP is older than about 3 s, or there are sequence gaps.
  - Clock offset above 50 ms.
  - Geoblock returns `blocked:true`.
  - Market config differs from the expected fingerprint (fee, delay, TWAP lookback, tick).
  - A balance or position reconciliation mismatch.
  - A manual kill from the dashboard.

  The kill switch cancels everything, stops new orders and sends an alert.
- **Order safety:**
  - Timeouts are longer than 150 ms + round trip, with margin.
  - After a timeout, query open orders and trades **before** any retry. Blind retries risk a double fill.
  - Each order runs through its own state machine to CONFIRMED or FAILED (trades go MATCHED → MINED → CONFIRMED, and can also go RETRYING → FAILED).
  - Reconcile with the Data API after every window and on every reconnect.
  - Stop placing new orders at T-X s by our own clock.
  - Cancel resting orders at the window end.
  - Handle HTTP 425 (engine restart, post-only period), 429 and 503.
- **No self-trading:** two strategies or accounts must never be able to trade against each other. Polymarket's integrity rules ban wash trading and spoofing.

---

## 7. Security and secrets

- **Never paste keys into chat** (with Claude or anyone else). Never commit them. Never log them.
- **Paper mode (M1–M3) needs only the CLOB API credentials**, not the wallet private key. Derive them once, on a trusted machine. These credentials can't place orders, because orders need a signature from the private key, but they *can* cancel orders, so they are still secret.
- **Live mode:** prefer Polymarket **Session Keys**. They can trade but not withdraw, and they can be revoked. They are in beta, need a Builder API key and manual approval, and aren't in the Rust SDK yet, so **apply early**. Until then, keep only a small trading float in the account the bot controls.
- **Secrets storage:**
  - Load secrets at runtime from a file **outside the repo**, an encrypted store (sops/age) or the OS keyring.
  - `.env*` is gitignored.
  - Hold secrets in `secrecy` types so they never appear in `Debug` output or logs.
- **Supply chain:** pin crate versions and run `cargo deny` / `cargo audit`. Use only the official Polymarket crate; fake "polymarket" packages stole keys in 2026.
- **Exposure:** the dashboard and API are never on the public internet.

---

## 8. Hosting, legal and money (the user's situation)

- **The user is in Serbia.** Serbia is **not** geoblocked; a live check returned `blocked:false`, `RS`.
- **Hosting, now: the user's local machine in Serbia.**
  - Both the Rust server and the dashboard run locally. Start the app and it records and trades; stop it and everything stops.
  - On startup, and before trading resumes after a stop, the bot reconciles open positions and redeems anything that resolved while it was off.
  - If the machine sleeps, network and timers pause and data goes stale. The bot must detect a wake-up from sleep, stop trading until feeds are fresh, and log the gap. Keep the machine awake while running (macOS: `caffeinate`).
  - **Clock:** rely on the OS time sync, but measure the offset against an NTP server at startup and periodically, and refuse to trade if it exceeds the threshold (§6).
- **Hosting, later: cloud (the user's preference is Ireland, close to Polymarket).**
  - Polymarket's servers are in AWS London (eu-west-2), but the UK is close-only, so London itself is out. The developer docs name **Ireland (eu-west-1)** as the closest allowed region.
  - However, the help centre lists Ireland (and the Netherlands) as restricted, so **Ireland's eligibility must be confirmed** (Polymarket support plus a live geoblock check from the VPS) before committing to it. Fallbacks: Finland, Sweden or Austria.
  - **Avoid** the UK, Germany (including Hetzner DE), France, Italy, Poland, Belgium, the US and Singapore.
- **Never use a VPN.** It breaks the ToS and leads to account freezes.
- Call `GET polymarket.com/api/geoblock` at startup and every few minutes, and refuse to trade if `blocked:true`.
- **Money flow:**
  - Card deposit through MoonPay (about 2–4% fee).
  - The Polymarket balance is held as pUSD in the account's wallet.
  - Getting money to a **Serbian bank is likely unsupported**. A Visa card payout is the probable route (about 3–5%). M0 must test this with $20.
  - Withdraw rarely and in larger amounts; card-in plus bank-out fees could exceed the trading edge.
- **Legal and tax (not legal advice):**
  - Serbia's Law on Games of Chance (Art. 10) restricts taking part in foreign games of chance. It hasn't been tested whether this covers prediction markets.
  - Profits may be taxed at 15% (digital-asset capital gains) or 20% (other income), via quarterly self-assessment.
  - **Book a Serbian tax adviser before M4.**
  - The ledger records every fill, fee, redemption, deposit and withdrawal with its USD value and its **RSD value at the NBS middle rate**, so tax reporting is mechanical.

---

## 9. Platform-change discipline

Polymarket changed fees three times in 2026. It also changed the taker delay (250 → 50 → 150 ms), the TWAP window (30 → 60 s), the API version (CLOB V1 → V2, with pUSD as collateral) and the price feeds (RTDS → PolyBolt). Some doc pages are stale.

- **Never hard-code** fee rates, tick size, minimum size, taker delay or TWAP lookback. Read them from each market's config at runtime.
- Compute a **config fingerprint** per market and halt trading on any unexpected change.
- When touching integration code, check `docs.polymarket.com/changelog` first. The changelog is more current than the topic pages.

---

## 10. Working agreements for Claude in this repo

- Read the relevant spec in `docs/specs/` before implementing. If the code and this file disagree, ask; don't guess.
- Don't read `.env*`, key files or other secret material (see the org security rules). Ask the user instead.
- Never enable or exercise live trading, sign real orders or move funds unless the user explicitly asks in that session.
- Window maths, TWAP estimation, the tie rule, fee rounding, the fill simulator and risk limits are **test-first, with deterministic replay tests**. These are where a silent bug costs money.
- Money and prices use `Decimal` and timestamps are UTC. Store the source and receive timestamps separately.
- Keep strategy code free of I/O so it runs the same in replay, paper and live.
- **Commands.** Run from the repo root unless noted. `README.md` ("Run M1 locally") has the details.
  - Run everything: `make dev` (Postgres, then traderd and the dashboard; Ctrl-C stops both, `make db-down` stops Postgres).
  - One at a time: `make db-up`, `make backend-run` (API on `127.0.0.1:8787`), `make frontend-dev` (dashboard on `http://localhost:3000`).
  - Build: `cargo build --release --manifest-path backend/Cargo.toml`; `cd frontend && pnpm build`.
  - Test everything: `make test` (backend `cargo test --workspace` plus frontend `vitest run`).
  - Backend only: `make backend-test`; lint with `cd backend && cargo clippy --workspace --all-targets -- -D warnings`.
  - Frontend only (in `frontend/`): `pnpm test`, `pnpm typecheck`, `pnpm build`.
  - End-to-end smoke (needs `make dev` running; first time `pnpm exec playwright install chromium`): `cd frontend && pnpm e2e`. Against the live backend it pauses and resumes a running bot, or tries to resume one the backend stopped itself (safety or startup); it never resumes a manual pause or a Kill, never acknowledges a stop reason, and never clicks Kill.
  - Regenerate the dashboard's API types after changing Rust DTOs: `cd backend && cargo test -p raki-core export_bindings` (writes `frontend/src/api/generated/`, never hand-edit it).
  - Inspect a raw file: `cargo run --release --manifest-path backend/Cargo.toml -p traderd --bin rawcat -- data/raw/<day>/<file>.ndjson.zst`.

---

## 11. Open questions (tracked; answer before the milestone that needs them)

- **M0:** Which wallet type does the user's account have, and who is the signer for an email or Google Deposit Wallet? Does a withdrawal to a Serbian bank or card actually work, and what does it cost?
- **M1:** Does PolyBolt's final TWAP value always equal Gamma's `finalPrice`? What is Chainlink's exact TWAP sampling? We'll estimate it empirically.
- **M1:** When will the legacy RTDS feed actually shut off? It doesn't matter if we build on PolyBolt only.
- **M2:** How often does a fillable 0.92–0.97 ask exist at T-45 to T-30 s, and how often do those windows reverse?
- **M2:** Does the realistic simulator show any edge for v0 or its variants?
- **M4:** Is the session-key application approved? Can a Serbian individual get co-location? That's optional, since speed isn't our edge.
- **M4:** What is the real starting balance for live trading? It sets the loss caps and drawdown limits.
- **Cloud move:** is Ireland (eu-west-1) actually allowed? The docs and the help centre disagree.
- **Before M4:** what is the tax classification in Serbia, and what's the legal view on the Law on Games of Chance?

## 12. Glossary

- **Window:** one 5-minute market, aligned to Unix multiples of 300 s.
- **Price to beat:** the TWAP reference at the window start; equal to the previous window's final TWAP.
- **TWAP:** time-weighted average price. Here it is Chainlink's 60-second average, which decides settlement.
- **Taker / maker:**
  - A taker order crosses the spread: it pays the fee and is held 150 ms.
  - A maker order rests on the book: no fee, and it earns a rebate.
- **FAK / FOK:** fill-and-kill (partial fills allowed) / fill-or-kill (all or nothing).
- **Redeem:** converting winning shares into pUSD after resolution.
- **pUSD:** Polymarket's collateral token, a USD stablecoin.
- **Deposit Wallet:** the wallet type for accounts created after May 2026 (signature type 3).
- **PolyBolt:** Polymarket's real-time price WebSocket.
- **Gamma:** the market-discovery API.
- **CLOB:** the order book and matching API.
- **Go/no-go gate:** the pre-agreed numbers that must pass before real money is risked (§5).

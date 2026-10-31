# Shadow predictions (pre-M2): live model probabilities and strategy v0 decisions, no orders

Status: 2026-10-31, approved direction ("both": M1 hardening, then shadow predictions).

## Goal

While the recorder runs, compute a model probability for every live window and log what strategy
v0 (CLAUDE.md §3) *would* have done for BTC, including a simulated fill. This shows early whether
the model is calibrated and how often v0 finds a fillable entry. **It places no orders and needs no
private key.**

**What it is not.** Mode is `shadow`: never `paper` or `live`. Shadow results never count toward
the M2 gate or the M3 go/no-go gate (backtest and pre-gate trades never count, §5). The model and
parameters here are a first hypothesis; M2 replaces or tunes them on the tuning split only.

## 1. What decides settlement (measured 2026-10-31)

PolyBolt `price.crypto.twap` vs `price.crypto` (1 s Chainlink spot), 645 points per asset:

- About 78% of TWAP points equal, to 1e-12 relative, the plain mean of the 60 one-second spot
  prints ending 2 or 3 s before the TWAP's timestamp (lag 2 and lag 3 about equally often), or
  repeat the previous TWAP value. The rest differ by a few cents to under $1 on BTC (fractional
  edge weighting or report timing; not pinned down).
- PolyBolt and the legacy RTDS TWAP were bit-identical on all 249 shared points, and our PolyBolt
  final TWAP equalled Gamma `finalPrice` exactly in the first PolyBolt windows.
- Consequence: the final TWAP is fixed about 2–3 s before the window end; the spot prints that
  matter are those in roughly `[end − 62, end − 2]`.

## 2. Model `twap-gauss-v0`

Pure function in `raki-strategy::model`. Inputs at time `t` (unix s) for a window ending at `T`:
price to beat `ptb`, the Chainlink spot points we hold (1 s), parameters below.

- **Averaging set:** the 60 seconds `k ∈ [T − lag − 59, T − lag]`, `lag = twap_lag_s` (default 2).
- **Observed part:** spot prints we already have for `k ≤ t`. A missing second inside the
  observed part is filled with the previous print (Chainlink publishes the last value).
- **Unknown part:** `k > t`, offsets `d_i = k_i − t` (1, 2, …). Modelled as a driftless random
  walk from the latest spot `S_t` with per-second standard deviation `σ`.
- **Final TWAP** `F ~ Normal(μ, sd²)`:
  - `μ = (Σ observed + n_unknown · S_t) / 60`
  - `sd² = σ² · Σ_i Σ_j min(d_i, d_j) / 60² + resid²`; for sorted `d`,
    `Σ_i Σ_j min(d_i, d_j) = Σ_i d_i · (2(n − i) + 1)` with `i` 1-based.
  - `resid = resid_bps · 1e-4 · S_t` covers the unexplained 22% (default 0.1 bp ≈ $0.8 on BTC).
- **σ:** sample standard deviation of 1 s spot differences over the last `vol_lookback_s` (default
  300), times `vol_mult` (default 1.0). Needs at least `vol_min_points` (default 120) differences;
  otherwise no prediction.
- **P(Up)** `= P(F ≥ ptb) = Φ((μ − ptb) / sd)` (a tie resolves Up; with `sd = 0` it is 1 when
  `μ ≥ ptb`, else 0). `P(Down) = 1 − P(Up)`.
- `ptb`: our recorded TWAP at the window start, else Gamma's `priceToBeat`; none → no prediction.
- Known weakness, to be measured not assumed away: Gaussian tails are thin, so 0.95+ predictions
  are the most likely to be over-confident. Calibration (§6) is the check.

Floating point is fine for the model (oracle values are f64 on the wire). Money (prices paid,
fees, stake, PnL) is `Decimal`.

## 3. Predictions (all assets)

At checkpoints `secs_left ∈ {240, 180, 120, 90, 60, 45, 40, 35, 30, 20, 10, 5}` (the first tick
at or below each) the engine stores one row per live window: `p_up`, `μ`, `sd`, `σ`, `ptb`, the
latest TWAP and spot, Binance and Coinbase mids, best bid/ask of both sides. Outcome comes from
`windows.winner` at query time.

## 4. Strategy v0 shadow (BTC only)

Parameters (`strategy_id = "v0"`, versioned by a hash of the parameter set):

| Parameter | Default | Note |
|---|---|---|
| `entry_window_start_s` / `entry_window_end_s` | 45 / 30 | evaluate while `30 ≤ secs_left ≤ 45` |
| `min_market_prob` | 0.92 | side's mid `(bid + ask) / 2` |
| `max_entry_price` | 0.97 | my call; v0 targets 0.92–0.97 asks |
| `min_edge` | 0.01 | my call, the §5 minimum worthwhile edge |
| `stake_usd` | 5 | user's decision |
| `sim_latency_ms` | 250 | 150 ms taker delay + an assumed 100 ms round trip (measure in M4) |

Every 1 s tick in the entry window, until a fill:

1. **Gates**, each a skip reason: control not `running` or a safety check failing; no prediction;
   market config not as expected (Gamma `feeSchedule.rate` missing, `exponent ≠ 1`, tick size or
   `orderMinSize` missing); no book for either side.
2. **Side:** the side whose mid is `≥ min_market_prob` (none → skip `no favourite`).
3. **Walk the asks** of that side up to `max_entry_price`, buying until `stake_usd` is spent
   (last level partially). Shares below `orderMinSize` → skip `not enough liquidity ≤ max`.
4. **Fee per fill** `= shares · rate · p · (1 − p)` per level, `rate` from Gamma. Not rounded
   (Polymarket's rounding is unverified; check against the first real fill in M4).
5. **Edge** `= model_prob(side) − (avg_price + fee / shares)`. Below `min_edge` → skip `edge`.
6. **Intent:** FAK buy of that side, notional `stake_usd`, limit = the highest price level used in
   step 3. One intent at a time.
7. **Simulated fill**, due `sim_latency_ms` after the decision: on the first book event or tick at
   or after the due time, walk that side's asks as they are then, up to the limit and the stake.
   Result: `filled` (stake spent), `partial`, or `no_fill` (book moved past the limit). After a
   `no_fill` a later tick may try again; after any fill the window is done (one position per
   window).

Every evaluation is stored (action `enter` or `skip` with its reason) so the "wanted to trade but
couldn't" rate and each skip reason's frequency are measurable. PnL per fill at resolution:
`shares − cost − fee` if the side won, else `−(cost + fee)`.

## 5. Storage (Postgres, migration 0003)

- `predictions(slug, checkpoint_s, ts_ms, p_up, mu, sd, sigma, ptb, twap, spot, binance_mid,
  coinbase_mid, up_bid, up_ask, down_bid, down_ask, model_id, session_id)`, key `(slug, checkpoint_s)`.
- `shadow_evals(id, session_id, strategy_id, params_hash, slug, ts_ms, secs_left, side, model_prob,
  market_mid, best_ask, walk_shares, walk_avg_price, walk_fee, edge, action, reason)`.
- `shadow_fills(eval_id, slug, side, due_ms, filled_ms, limit_price, shares, cost, fee, status)`.
- `strategy_params(params_hash, strategy_id, params jsonb, first_seen_ms)`.
- `run_id` = the session id; `mode` = `shadow` is implied by these tables.

## 6. API and dashboard

- `GET /api/shadow/summary`: evaluations, entries, fills by status, no-fill rate, wins, losses,
  PnL, mean predicted edge vs realized edge per fill; skip reasons with counts.
- `GET /api/shadow/calibration?checkpoint=30`: buckets `<0.5, 0.5–0.8, 0.8–0.92, 0.92–0.95,
  0.95–0.97, 0.97–0.99, ≥0.99` of `max(p_up, 1 − p_up)`: n, mean predicted, realized rate, Wilson
  95% interval. Resolved windows only.
- `GET /api/windows/{slug}/predictions` and `/shadow` for the window page.
- The live snapshot carries each live window's latest `p_up` and BTC's latest v0 evaluation.
- Dashboard: model P(Up) and the v0 status on the live cards; a **Shadow** page with the summary,
  calibration table and recent fills, labelled "shadow, not a gate"; predictions and evaluations on
  the window page.

## 7. Tests (test-first)

- Model: the `min` sum identity; `sd = 0` and the tie rule; a window fully observed; known σ and a
  constructed path give the closed-form μ and sd; P(Up) symmetric around `ptb`; missing seconds
  forward-filled; too few points → none.
- Book walk and fee: stake split across levels, partial last level, limit cut-off, min size, fee
  per level with exact `Decimal` values.
- v0: each skip reason, side choice, edge threshold, one position per window, retry after no fill,
  fill against the book at the due time (not at the decision).
- Store and API: insert and summary/calibration queries on fixed data.

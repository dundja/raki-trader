//! `twap-gauss-v0`: the probability that a window settles Up, from the observed part of its final
//! TWAP and a random-walk forecast of the rest (docs/specs/2026-10-31-shadow-predictions.md §2).

use serde::Serialize;

pub const MODEL_ID: &str = "twap-gauss-v0";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelParams {
    /// The final TWAP averages the spot prints ending this many seconds before the window end.
    pub twap_lag_s: i64,
    /// σ is measured over this many seconds back from the latest print...
    pub vol_lookback_s: i64,
    /// ...and needs at least this many one-second differences.
    pub vol_min_points: usize,
    pub vol_mult: f64,
    /// Unexplained TWAP noise, in basis points of the spot price.
    pub resid_bps: f64,
}

impl Default for ModelParams {
    fn default() -> ModelParams {
        ModelParams {
            twap_lag_s: 2,
            vol_lookback_s: 300,
            vol_min_points: 120,
            vol_mult: 1.0,
            resid_bps: 0.1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prediction {
    pub p_up: f64,
    /// Mean and standard deviation of the forecast final TWAP.
    pub mu: f64,
    pub sd: f64,
    /// Per-second spot volatility used (after `vol_mult`).
    pub sigma: f64,
    /// The latest spot print the forecast starts from: `(unix s, value)`.
    pub spot_ts: i64,
    pub spot: f64,
}

/// Forecasts the final TWAP of a window ending at `end_s` and returns `P(final ≥ ptb)`.
///
/// `spot` holds one-second Chainlink prints `(unix s, value)`, ascending, one per second at
/// most; a second without a print carries the previous value. `lookback_s` is the market's TWAP
/// lookback (60 s today; read from its config, never assumed). `None` when the spot history is
/// empty, too short for σ, or does not reach back to the first observed second.
pub fn predict(
    params: &ModelParams,
    ptb: f64,
    end_s: i64,
    lookback_s: i64,
    spot: &[(i64, f64)],
) -> Option<Prediction> {
    let &(t, s_t) = spot.last()?;
    if lookback_s <= 0 {
        return None;
    }
    let sigma = sigma(spot, t, params)?;
    let last_k = end_s - params.twap_lag_s;
    let first_k = last_k - lookback_s + 1;

    let mut observed = 0.0;
    let mut i = 0;
    let mut carried = None;
    for k in first_k..=last_k.min(t) {
        while i < spot.len() && spot[i].0 <= k {
            carried = Some(spot[i].1);
            i += 1;
        }
        observed += carried?;
    }

    let d0 = (first_k.max(t + 1) - t).max(1);
    let n = (last_k - t).max(0).min(lookback_s);
    let d_last = d0 + n - 1;
    let min_sum = consecutive_min_sum(d0, d_last);

    let len = lookback_s as f64;
    let mu = (observed + n as f64 * s_t) / len;
    let resid = params.resid_bps * 1e-4 * s_t;
    let sd = (sigma * sigma * min_sum / (len * len) + resid * resid).sqrt();
    Some(Prediction {
        p_up: p_at_least(mu, sd, ptb),
        mu,
        sd,
        sigma,
        spot_ts: t,
        spot: s_t,
    })
}

/// `P(X ≥ x)` for `X ~ Normal(mu, sd²)`. A degenerate forecast is certain: a tie is Up.
fn p_at_least(mu: f64, sd: f64, x: f64) -> f64 {
    if sd <= 0.0 {
        return if mu >= x { 1.0 } else { 0.0 };
    }
    0.5 * libm::erfc((x - mu) / (sd * std::f64::consts::SQRT_2))
}

/// RMS of one-second differences ending at `t`, over the lookback, times `vol_mult`.
fn sigma(spot: &[(i64, f64)], t: i64, params: &ModelParams) -> Option<f64> {
    let from = t - params.vol_lookback_s;
    let (sum_sq, n) = spot
        .windows(2)
        .filter(|w| w[1].0 - w[0].0 == 1 && w[0].0 >= from)
        .map(|w| (w[1].1 - w[0].1).powi(2))
        .fold((0.0, 0usize), |(s, n), sq| (s + sq, n + 1));
    (n >= params.vol_min_points.max(1)).then(|| (sum_sq / n as f64).sqrt() * params.vol_mult)
}

/// `Σ_i Σ_j min(d_i, d_j)` over the offsets `a..=b` (0 if empty): the variance factor of a mean
/// of random-walk points. For sorted offsets it is `Σ_i d_i · (2(n − i) + 1)`, `i` 1-based.
fn consecutive_min_sum(a: i64, b: i64) -> f64 {
    if b < a {
        return 0.0;
    }
    let n = b - a + 1;
    (1..=n)
        .map(|i| ((a + i - 1) * (2 * (n - i) + 1)) as f64)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const END: i64 = 1_000_000;

    fn params() -> ModelParams {
        ModelParams {
            resid_bps: 0.0,
            vol_min_points: 10,
            ..ModelParams::default()
        }
    }

    /// One print per second over `[from, to]`, alternating `base ± 0.5`, so every one-second
    /// difference is ±1 and σ is exactly 1.
    fn zigzag(from: i64, to: i64, base: f64) -> Vec<(i64, f64)> {
        (from..=to)
            .map(|k| (k, if k % 2 == 0 { base + 0.5 } else { base - 0.5 }))
            .collect()
    }

    #[test]
    fn min_sum_matches_brute_force() {
        for (a, b) in [(1, 1), (1, 28), (3, 10), (31, 90), (5, 4)] {
            let d: Vec<i64> = (a..=b).collect();
            let brute: i64 = d.iter().flat_map(|&x| d.iter().map(move |&y| x.min(y))).sum();
            assert_eq!(consecutive_min_sum(a, b), brute as f64, "{a}..={b}");
        }
        // Σ_{i,j ≤ n} min(i, j) = n(n + 1)(2n + 1) / 6.
        assert_eq!(consecutive_min_sum(1, 28), (28 * 29 * 57 / 6) as f64);
    }

    #[test]
    fn fully_observed_window_is_certain_and_a_tie_is_up() {
        let spot: Vec<(i64, f64)> = (END - 400..=END + 5).map(|k| (k, 100.0)).collect();
        let mut p = params();
        p.vol_min_points = 1;
        let at = |ptb| predict(&p, ptb, END, 60, &spot).unwrap();
        let tie = at(100.0);
        assert_eq!((tie.mu, tie.sd, tie.sigma), (100.0, 0.0, 0.0));
        assert_eq!(tie.p_up, 1.0, "a tie resolves Up");
        assert_eq!(at(100.000_001).p_up, 0.0);
    }

    #[test]
    fn mean_and_sd_follow_the_closed_form() {
        // Latest print 30 s before the end: observed seconds END-61..=END-30 (32), unknown
        // END-29..=END-2 (28, offsets 1..=28).
        let t = END - 30;
        let spot = zigzag(t - 300, t, 100.0);
        let pred = predict(&params(), 100.0, END, 60, &spot).unwrap();
        assert_eq!(pred.sigma, 1.0);
        assert_eq!(pred.spot_ts, t);
        let observed: f64 = spot
            .iter()
            .filter(|(k, _)| (END - 61..=t).contains(k))
            .map(|(_, v)| v)
            .sum();
        let expected_mu = (observed + 28.0 * pred.spot) / 60.0;
        assert!((pred.mu - expected_mu).abs() < 1e-12);
        let expected_sd = (7714.0_f64 / 3600.0).sqrt();
        assert!((pred.sd - expected_sd).abs() < 1e-12, "{} vs {expected_sd}", pred.sd);
    }

    #[test]
    fn before_the_averaging_starts_every_second_is_unknown() {
        // 120 s before the end: offsets 59..=118, nothing observed.
        let t = END - 120;
        let spot = zigzag(t - 300, t, 100.0);
        let pred = predict(&params(), 0.0, END, 60, &spot).unwrap();
        assert!((pred.mu - pred.spot).abs() < 1e-12);
        let expected_sd = (consecutive_min_sum(59, 118) / 3600.0).sqrt();
        assert!((pred.sd - expected_sd).abs() < 1e-12);
    }

    #[test]
    fn probability_is_symmetric_around_the_mean() {
        let spot = zigzag(END - 330, END - 30, 100.0);
        let mu = predict(&params(), 0.0, END, 60, &spot).unwrap().mu;
        let up = predict(&params(), mu - 0.7, END, 60, &spot).unwrap().p_up;
        let down = predict(&params(), mu + 0.7, END, 60, &spot).unwrap().p_up;
        assert!(up > 0.5 && down < 0.5);
        assert!((up + down - 1.0).abs() < 1e-12);
        let at_mu = predict(&params(), mu, END, 60, &spot).unwrap().p_up;
        assert!((at_mu - 0.5).abs() < 1e-12);
    }

    #[test]
    fn a_missing_second_carries_the_previous_print() {
        let full = zigzag(END - 330, END - 30, 100.0);
        let mut holed = full.clone();
        let gap = END - 50;
        holed.retain(|(k, _)| *k != gap);
        let mut filled = full.clone();
        let prev = filled.iter().find(|(k, _)| *k == gap - 1).unwrap().1;
        filled.iter_mut().find(|(k, _)| *k == gap).unwrap().1 = prev;
        let a = predict(&params(), 100.0, END, 60, &holed).unwrap();
        let b = predict(&params(), 100.0, END, 60, &filled).unwrap();
        assert!((a.mu - b.mu).abs() < 1e-12);
    }

    #[test]
    fn residual_noise_keeps_a_settled_window_uncertain_near_the_line() {
        let spot: Vec<(i64, f64)> = (END - 400..=END).map(|k| (k, 100.0)).collect();
        let p = ModelParams {
            vol_min_points: 1,
            resid_bps: 1.0,
            ..ModelParams::default()
        };
        let pred = predict(&p, 100.0, END, 60, &spot).unwrap();
        assert!((pred.sd - 0.01).abs() < 1e-12);
        assert!((pred.p_up - 0.5).abs() < 1e-12);
    }

    #[test]
    fn insufficient_history_gives_no_prediction() {
        assert_eq!(predict(&params(), 1.0, END, 60, &[]), None);
        // Too few differences for σ.
        let short = zigzag(END - 35, END - 30, 100.0);
        assert_eq!(predict(&params(), 1.0, END, 60, &short), None);
        // σ is fine, but the history starts after the first observed second (END - 61).
        let late = zigzag(END - 55, END - 30, 100.0);
        assert_eq!(predict(&params(), 1.0, END, 60, &late), None);
        assert_eq!(predict(&params(), 1.0, END, 0, &zigzag(END - 330, END - 30, 1.0)), None);
    }
}

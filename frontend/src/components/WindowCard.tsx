import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { useEffect, useMemo, useState } from 'react'

import { getJson, qk } from '../api/client'
import type { LiveWindow } from '../api/generated/LiveWindow'
import type { SeriesRow } from '../api/generated/SeriesRow'
import { fmtAgo, fmtCountdown, fmtPct, fmtPrice, fmtProb, fmtSigned, fmtUtc } from './fmt'
import { shadowText } from './shadow'
import { useNow } from './useNow'

const COUNTDOWN_TICK_MS = 250
/** Live points kept for the sparkline; a window lasts 300 s and snapshots arrive about once a second. */
const MAX_LIVE_POINTS = 400

type Point = { t: number; v: number }

/** Seed once from /api/windows/{slug}/series, then append the TWAP of each pushed snapshot. */
function useTwapPoints(w: LiveWindow): Point[] {
  const seed = useQuery({
    queryKey: qk.series(w.slug),
    queryFn: () => getJson<SeriesRow[]>(`/api/windows/${w.slug}/series`),
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  })
  const [live, setLive] = useState<Point[]>([])

  useEffect(() => {
    if (w.twap === null || w.twap_ts_ms === null) return
    const point = { t: w.twap_ts_ms, v: w.twap }
    setLive((prev) => {
      const last = prev[prev.length - 1]
      if (last && last.t >= point.t) return prev
      return [...prev, point].slice(-MAX_LIVE_POINTS)
    })
  }, [w.twap, w.twap_ts_ms])

  return useMemo(() => {
    const seeded: Point[] = []
    for (const row of seed.data ?? []) {
      if (row.twap !== null) seeded.push({ t: row.ts * 1000, v: row.twap })
    }
    // live points win where the two overlap
    const firstLive = live[0]?.t ?? Infinity
    return [...seeded.filter((p) => p.t < firstLive), ...live]
  }, [seed.data, live])
}

const SPARK_W = 160
const SPARK_H = 36

/** Minimal TWAP line. Marks use the chart tokens; the axis is implicit (min..max of the points). */
function Sparkline({ points }: { points: Point[] }) {
  if (points.length < 2) {
    return (
      <div className="sparkline sparkline-empty" data-testid="sparkline" data-points={points.length}>
        —
      </div>
    )
  }
  const t0 = points[0].t
  const t1 = points[points.length - 1].t
  let lo = Infinity
  let hi = -Infinity
  for (const p of points) {
    lo = Math.min(lo, p.v)
    hi = Math.max(hi, p.v)
  }
  const dt = t1 - t0 || 1
  const dv = hi - lo || 1
  const d = points
    .map((p, i) => {
      const x = ((p.t - t0) / dt) * SPARK_W
      const y = SPARK_H - 2 - ((p.v - lo) / dv) * (SPARK_H - 4)
      return `${i === 0 ? 'M' : 'L'}${x.toFixed(1)} ${y.toFixed(1)}`
    })
    .join(' ')
  return (
    <svg
      className="sparkline"
      data-testid="sparkline"
      data-points={points.length}
      viewBox={`0 0 ${SPARK_W} ${SPARK_H}`}
      role="img"
      aria-label={`TWAP this window, ${fmtPrice(lo)} to ${fmtPrice(hi)}`}
    >
      <path d={d} fill="none" stroke="var(--series-twap)" strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="field">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}

function bookSide(bid: string | null, ask: string | null): string {
  return `${fmtProb(bid)} / ${fmtProb(ask)}`
}

/** One open window: countdown, TWAP vs price to beat, venue prices, the Up/Down book and a sparkline. */
export function WindowCard({ window: w }: { window: LiveWindow }) {
  const now = useNow(COUNTDOWN_TICK_MS)
  const points = useTwapPoints(w)

  const secsLeft = (w.end_ts * 1000 - now) / 1000
  const ended = secsLeft <= 0

  const delta = w.twap !== null && w.price_to_beat !== null ? w.twap - w.price_to_beat : null
  // Display only: a tie resolves Up, so zero counts as leading Up.
  const dir = delta === null ? undefined : delta >= 0 ? 'up' : 'down'
  const bp =
    delta !== null && w.price_to_beat !== null && w.price_to_beat !== 0
      ? (delta / w.price_to_beat) * 10_000
      : null

  return (
    <article className="card window-card" data-testid={`window-card-${w.asset}`}>
      <header className="card-head">
        <h2>{w.asset.toUpperCase()}</h2>
        <Link to={`/windows/${w.slug}` as '/'} className="slug">
          {w.slug}
        </Link>
        <span
          className={ended ? 'countdown countdown-ended' : 'countdown'}
          data-testid="countdown"
          title={`ends ${fmtUtc(w.end_ts * 1000)}`}
        >
          {ended ? 'ended, awaiting resolution' : fmtCountdown(secsLeft)}
        </span>
      </header>

      <dl className="fields">
        <Field label="Price to beat">{fmtPrice(w.price_to_beat)}</Field>
        <Field label="TWAP">{fmtPrice(w.twap)}</Field>
        <Field label="TWAP − PTB">
          <span className="delta" data-testid="twap-delta" data-dir={dir}>
            {delta === null ? (
              '—'
            ) : (
              <>
                {fmtSigned(delta)} USD · {fmtSigned(bp, 1)} bp
              </>
            )}
          </span>
        </Field>
        <Field label="Chainlink">{fmtPrice(w.chainlink)}</Field>
        <Field label="Binance (USD)">{fmtPrice(w.binance_mid_usd)}</Field>
        <Field label="Coinbase">{fmtPrice(w.coinbase_mid)}</Field>
        <Field label="Up bid / ask">{bookSide(w.up_bid, w.up_ask)}</Field>
        <Field label="Down bid / ask">{bookSide(w.down_bid, w.down_ask)}</Field>
        <Field label="Model P(Up)">
          <span
            data-testid="model-p-up"
            title={
              w.prediction === null
                ? undefined
                : `final TWAP forecast ${fmtPrice(w.prediction.mu)} ± ${fmtPrice(w.prediction.sd)}`
            }
          >
            {fmtPct(w.prediction?.p_up)}
          </span>
        </Field>
        {w.shadow !== null && (
          <Field label="v0 shadow">
            <span data-testid="v0-shadow">{shadowText(w.shadow)}</span>
          </Field>
        )}
        <Field label="TWAP age">
          <span title={w.twap_ts_ms === null ? undefined : fmtUtc(w.twap_ts_ms)}>
            {fmtAgo(w.twap_ts_ms, now)}
          </span>
        </Field>
      </dl>

      <Sparkline points={points} />
    </article>
  )
}

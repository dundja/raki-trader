import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'

import type { SeriesRow } from '../api/generated/SeriesRow'
import { fmtPrice } from './fmt'

/** A window lasts 300 s; x is seconds into it. */
export const WINDOW_SECS = 300
const X_TICKS = [0, 60, 120, 180, 240, 300]

export type ChartPoint = {
  x: number
  twap: number | null
  chainlink: number | null
  binance: number | null
  coinbase: number | null
}

export type QuotePoint = { x: number; bid: number | null; ask: number | null }

function inWindow(x: number): boolean {
  return Number.isFinite(x) && x >= 0 && x <= WINDOW_SECS
}

/** Series rows → chart points with x = seconds into the window. Samples outside 0..300 s are dropped. */
export function toChartPoints(rows: readonly SeriesRow[], startTs: number): ChartPoint[] {
  const out: ChartPoint[] = []
  for (const r of rows) {
    const x = r.ts - startTs
    if (!inWindow(x)) continue
    out.push({
      x,
      twap: r.twap,
      chainlink: r.chainlink,
      binance: r.binance_mid_usd,
      coinbase: r.coinbase_mid,
    })
  }
  return out
}

/** A decimal string (0..1) as cents for plotting only; null/invalid stays null. */
function cents(v: string | null): number | null {
  if (v === null || v === '') return null
  const n = Number(v)
  // toFixed trims float noise (0.56 * 100 = 56.00000000000001)
  return Number.isFinite(n) ? Number((n * 100).toFixed(4)) : null
}

/** Up bid/ask per second as cents. */
export function toQuotePoints(rows: readonly SeriesRow[], startTs: number): QuotePoint[] {
  const out: QuotePoint[] = []
  for (const r of rows) {
    const x = r.ts - startTs
    if (!inWindow(x)) continue
    out.push({ x, bid: cents(r.up_bid), ask: cents(r.up_ask) })
  }
  return out
}

const CHART_HEIGHT = 300
const QUOTE_HEIGHT = 180
const AXIS = { stroke: 'var(--chart-axis)', fontSize: 12 } as const

type PriceKey = 'twap' | 'chainlink' | 'binance' | 'coinbase'

// TWAP is the primary series (thicker); the others are secondary. Order = categorical slot order.
const PRICE_LINES: ReadonlyArray<{ key: PriceKey; name: string; color: string; width: number }> = [
  { key: 'twap', name: 'TWAP', color: 'var(--series-twap)', width: 2.5 },
  { key: 'chainlink', name: 'Chainlink', color: 'var(--series-chainlink)', width: 1.5 },
  { key: 'binance', name: 'Binance (USD)', color: 'var(--series-binance)', width: 1.5 },
  { key: 'coinbase', name: 'Coinbase', color: 'var(--series-coinbase)', width: 1.5 },
]

/** y-domain that fits every plotted price and the reference line, with a little padding. */
function priceDomain(points: readonly ChartPoint[], ref: number | null): [number, number] {
  let lo = Infinity
  let hi = -Infinity
  const take = (v: number | null) => {
    if (v === null || !Number.isFinite(v)) return
    if (v < lo) lo = v
    if (v > hi) hi = v
  }
  for (const p of points) {
    take(p.twap)
    take(p.chainlink)
    take(p.binance)
    take(p.coinbase)
  }
  take(ref)
  if (lo > hi) return [0, 1]
  const pad = Math.max((hi - lo) * 0.08, Math.abs(hi) * 1e-5, 0.01)
  return [lo - pad, hi + pad]
}

const tooltipStyle = {
  background: 'var(--surface)',
  border: '1px solid var(--border)',
  borderRadius: 6,
  color: 'var(--text)',
}

function secs(v: unknown): string {
  return `${String(v)} s`
}

type Props = {
  series: readonly SeriesRow[]
  startTs: number
  /** Price to beat: ours, falling back to Gamma. Drawn as a dashed reference line. */
  priceToBeat: number | null
}

/** Price chart (TWAP, Chainlink, Binance, Coinbase, dashed price-to-beat) plus an Up bid/ask chart. */
export function WindowChart({ series, startTs, priceToBeat }: Props) {
  const points = toChartPoints(series, startTs)
  const quotes = toQuotePoints(series, startTs) // same filter as points, so index i lines up
  const domain = priceDomain(points, priceToBeat)

  return (
    <div className="charts">
      <figure className="chart" data-testid="price-chart">
        <figcaption>Price vs seconds into the window (USD)</figcaption>
        <ResponsiveContainer width="100%" height={CHART_HEIGHT} initialDimension={{ width: 800, height: CHART_HEIGHT }}>
          <LineChart data={points} margin={{ top: 8, right: 16, bottom: 4, left: 8 }}>
            <CartesianGrid stroke="var(--chart-grid)" vertical={false} />
            <XAxis
              type="number"
              dataKey="x"
              domain={[0, WINDOW_SECS]}
              ticks={X_TICKS}
              tickFormatter={(v: number) => `${v}s`}
              {...AXIS}
            />
            <YAxis
              type="number"
              domain={domain}
              tickFormatter={(v: number) => fmtPrice(v, 0)}
              width={64}
              {...AXIS}
            />
            <Tooltip
              contentStyle={tooltipStyle}
              cursor={{ stroke: 'var(--chart-axis)' }}
              labelFormatter={secs}
              formatter={(v) => fmtPrice(typeof v === 'number' ? v : null)}
              isAnimationActive={false}
            />
            <Legend />
            {priceToBeat !== null && (
              <ReferenceLine
                y={priceToBeat}
                stroke="var(--series-ref)"
                strokeDasharray="6 4"
                strokeWidth={1.5}
                ifOverflow="extendDomain"
                label={{ value: 'price to beat', position: 'insideTopRight', fill: 'var(--text-muted)', fontSize: 12 }}
              />
            )}
            {PRICE_LINES.map((l) => (
              <Line
                key={l.key}
                type="monotone"
                dataKey={l.key}
                name={l.name}
                stroke={l.color}
                strokeWidth={l.width}
                dot={false}
                activeDot={{ r: 4, stroke: 'var(--surface)', strokeWidth: 2 }}
                connectNulls
                isAnimationActive={false}
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </figure>

      <figure className="chart" data-testid="quote-chart">
        <figcaption>Up bid / ask (¢)</figcaption>
        <ResponsiveContainer width="100%" height={QUOTE_HEIGHT} initialDimension={{ width: 800, height: QUOTE_HEIGHT }}>
          <LineChart data={quotes} margin={{ top: 8, right: 16, bottom: 4, left: 8 }}>
            <CartesianGrid stroke="var(--chart-grid)" vertical={false} />
            <XAxis
              type="number"
              dataKey="x"
              domain={[0, WINDOW_SECS]}
              ticks={X_TICKS}
              tickFormatter={(v: number) => `${v}s`}
              {...AXIS}
            />
            <YAxis
              type="number"
              domain={[0, 100]}
              tickFormatter={(v: number) => `${v}¢`}
              width={64}
              {...AXIS}
            />
            <Tooltip
              contentStyle={tooltipStyle}
              cursor={{ stroke: 'var(--chart-axis)' }}
              labelFormatter={secs}
              formatter={(v) => (typeof v === 'number' ? `${v.toFixed(1)}¢` : '—')}
              isAnimationActive={false}
            />
            <Legend />
            <Line type="stepAfter" dataKey="bid" name="Up bid" stroke="var(--series-twap)" strokeWidth={2} dot={false} activeDot={{ r: 4, stroke: 'var(--surface)', strokeWidth: 2 }} connectNulls isAnimationActive={false} />
            <Line type="stepAfter" dataKey="ask" name="Up ask" stroke="var(--series-chainlink)" strokeWidth={1.5} dot={false} activeDot={{ r: 4, stroke: 'var(--surface)', strokeWidth: 2 }} connectNulls isAnimationActive={false} />
          </LineChart>
        </ResponsiveContainer>
      </figure>

      <details className="chart-table">
        <summary>Show data as a table</summary>
        <div className="table-scroll">
          <table className="table" data-testid="series-table">
            <thead>
              <tr>
                <th className="num">Second</th>
                <th className="num">TWAP</th>
                <th className="num">Chainlink</th>
                <th className="num">Binance (USD)</th>
                <th className="num">Coinbase</th>
                <th className="num">Up bid</th>
                <th className="num">Up ask</th>
              </tr>
            </thead>
            <tbody>
              {points.map((p, i) => (
                <tr key={p.x}>
                  <td className="num">{p.x}</td>
                  <td className="num">{fmtPrice(p.twap)}</td>
                  <td className="num">{fmtPrice(p.chainlink)}</td>
                  <td className="num">{fmtPrice(p.binance)}</td>
                  <td className="num">{fmtPrice(p.coinbase)}</td>
                  <td className="num">{fmtCents(quotes[i]?.bid ?? null)}</td>
                  <td className="num">{fmtCents(quotes[i]?.ask ?? null)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </div>
  )
}

function fmtCents(v: number | null): string {
  return v === null ? '—' : `${v.toFixed(1)}¢`
}

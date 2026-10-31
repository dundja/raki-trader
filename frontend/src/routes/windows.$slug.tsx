import { Link, createFileRoute } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'

import { getJson, qk } from '../api/client'
import type { PredictionRow } from '../api/generated/PredictionRow'
import type { SeriesRow } from '../api/generated/SeriesRow'
import type { WindowRecord } from '../api/generated/WindowRecord'
import type { WindowShadow } from '../api/generated/WindowShadow'
import { fmtCents, fmtLocal, fmtNum, fmtPct, fmtPrice, fmtProb, fmtUtc } from '../components/fmt'
import { skipLabel } from '../components/shadow'
import { WindowChart } from '../components/WindowChart'
import { FillsTable } from './shadow'
import { StatusBadge, MatchBadge, coveragePct, resolutionLatency } from './windows.index'

export const Route = createFileRoute('/windows/$slug')({
  component: WindowDetailRoute,
})

function WindowDetailRoute() {
  const { slug } = Route.useParams()
  return <WindowDetailPage slug={slug} />
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="field">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}

const dash = (v: string | number | null): string => (v === null ? '—' : String(v))

function Time({ ms }: { ms: number | null }) {
  if (ms === null) return <>—</>
  return <span title={fmtUtc(ms)}>{fmtLocal(ms)}</span>
}

export function WindowDetailPage({ slug }: { slug: string }) {
  const win = useQuery({
    queryKey: qk.window(slug),
    queryFn: () => getJson<WindowRecord>(`/api/windows/${slug}`),
  })
  const series = useQuery({
    queryKey: qk.series(slug),
    queryFn: () => getJson<SeriesRow[]>(`/api/windows/${slug}/series`),
  })

  const predictions = useQuery({
    queryKey: qk.predictions(slug),
    queryFn: () => getJson<PredictionRow[]>(`/api/windows/${slug}/predictions`),
  })
  const shadow = useQuery({
    queryKey: qk.windowShadow(slug),
    queryFn: () => getJson<WindowShadow>(`/api/windows/${slug}/shadow`),
  })

  const w = win.data
  const rows = series.data

  return (
    <section>
      <p>
        <Link to="/windows">← All windows</Link>
      </p>

      {win.isPending && <p className="muted">Loading window…</p>}
      {win.isError && (
        <p className="dialog-error" role="alert">
          Could not load window: {win.error.message}
        </p>
      )}

      {w && (
        <>
          <div className="card">
            <div className="card-head">
              <h2>{w.asset.toUpperCase()} window</h2>
              <span className="slug mono">{w.slug}</span>
              <StatusBadge status={w.status} />
            </div>
            <dl className="fields">
              <Field label="Start">
                <Time ms={w.start_ts * 1000} />
              </Field>
              <Field label="End">
                <Time ms={w.end_ts * 1000} />
              </Field>
              <Field label="Price to beat (ours)">{fmtPrice(w.price_to_beat_ours)}</Field>
              <Field label="Price to beat (Gamma)">{fmtPrice(w.price_to_beat_gamma)}</Field>
              <Field label="Final TWAP (ours)">{fmtPrice(w.final_twap_ours)}</Field>
              <Field label="Final price (Gamma)">{fmtPrice(w.final_price_gamma)}</Field>
              <Field label="Outcome (ours)">{dash(w.outcome_ours)}</Field>
              <Field label="Winner">{dash(w.winner)}</Field>
              <Field label="TWAP match">
                <MatchBadge match={w.twap_match} status={w.status} />
              </Field>
              <Field label="TWAP source">{dash(w.twap_source)}</Field>
              <Field label="Resolved at">
                <Time ms={w.resolved_at === null ? null : w.resolved_at * 1000} />
              </Field>
              <Field label="Resolution latency">{resolutionLatency(w)}</Field>
              <Field label="Coverage">
                {coveragePct(w)} ({w.coverage_secs} s)
              </Field>
              <Field label="Session">
                <span className="mono">{w.session_id}</span>
              </Field>
              <Field label="Event ID">{dash(w.event_id)}</Field>
              <Field label="Condition ID">
                <span className="mono">{dash(w.condition_id)}</span>
              </Field>
              <Field label="Up token">
                <span className="mono">{dash(w.up_token)}</span>
              </Field>
              <Field label="Down token">
                <span className="mono">{dash(w.down_token)}</span>
              </Field>
              <Field label="Updated">
                <Time ms={w.updated_ms} />
              </Field>
            </dl>
          </div>

          <div className="card chart-card">
            <h2>Series</h2>
            {series.isPending && <p className="muted">Loading series…</p>}
            {series.isError && (
              <p className="dialog-error" role="alert">
                Could not load series: {series.error.message}
              </p>
            )}
            {rows && rows.length === 0 && (
              <p className="muted">
                No series recorded for this window (app was not running).
              </p>
            )}
            {rows && rows.length > 0 && (
              <WindowChart
                series={rows}
                startTs={w.start_ts}
                priceToBeat={w.price_to_beat_ours ?? w.price_to_beat_gamma}
              />
            )}
          </div>

          {predictions.data && predictions.data.length > 0 && (
            <div className="card">
              <h2>Model predictions (shadow)</h2>
              <PredictionsTable rows={predictions.data} winner={w.winner} />
            </div>
          )}

          {shadow.data && shadow.data.evals.length > 0 && (
            <div className="card">
              <h2>Strategy v0 (shadow, no orders)</h2>
              <FillsTable fills={shadow.data.fills} />
              <EvalsTable shadow={shadow.data} />
            </div>
          )}
        </>
      )}
    </section>
  )
}

function PredictionsTable({
  rows,
  winner,
}: {
  rows: PredictionRow[]
  winner: WindowRecord['winner']
}) {
  return (
    <div className="table-scroll">
      <table className="table" data-testid="predictions">
        <thead>
          <tr>
            <th className="num">Seconds left</th>
            <th className="num">P(Up)</th>
            <th className="num">Forecast final TWAP</th>
            <th className="num">Price to beat</th>
            <th className="num">Spot</th>
            <th className="num">Up bid / ask</th>
            <th>Right?</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((p) => {
            const favourite = p.p_up >= 0.5 ? 'Up' : 'Down'
            return (
              <tr key={p.checkpoint_s}>
                <td className="num">{p.checkpoint_s}</td>
                <td className="num">{fmtPct(p.p_up)}</td>
                <td className="num">
                  {fmtPrice(p.mu)} ± {fmtPrice(p.sd)}
                </td>
                <td className="num">{fmtPrice(p.ptb)}</td>
                <td className="num">{fmtPrice(p.spot)}</td>
                <td className="num">
                  {fmtProb(p.up_bid)} / {fmtProb(p.up_ask)}
                </td>
                <td>{winner === null ? '—' : favourite === winner ? 'yes' : 'no'}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

function EvalsTable({ shadow }: { shadow: WindowShadow }) {
  return (
    <details className="chart-table">
      <summary>Every evaluation ({shadow.evals.length})</summary>
      <div className="table-scroll">
        <table className="table" data-testid="shadow-evals">
          <thead>
            <tr>
              <th className="num">Seconds left</th>
              <th>Decision</th>
              <th>Side</th>
              <th className="num">Model</th>
              <th className="num">Market</th>
              <th className="num">Best ask</th>
              <th className="num">Shares ≤ 97¢</th>
              <th className="num">Edge</th>
            </tr>
          </thead>
          <tbody>
            {shadow.evals.map((e) => (
              <tr key={`${e.session_id}-${e.seq}`}>
                <td className="num">{e.secs_left}</td>
                <td>{e.action === 'enter' ? 'Enter' : `Skip: ${skipLabel(e.reason)}`}</td>
                <td>{e.side ?? '—'}</td>
                <td className="num">{fmtPct(e.model_prob)}</td>
                <td className="num">{fmtProb(e.market_prob)}</td>
                <td className="num">{fmtProb(e.best_ask)}</td>
                <td className="num">{fmtNum(e.walk_shares)}</td>
                <td className="num">{fmtCents(e.edge)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </details>
  )
}

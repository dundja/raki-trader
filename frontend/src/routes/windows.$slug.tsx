import { Link, createFileRoute } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'

import { getJson, qk } from '../api/client'
import type { SeriesRow } from '../api/generated/SeriesRow'
import type { WindowRecord } from '../api/generated/WindowRecord'
import { fmtLocal, fmtPrice, fmtUtc } from '../components/fmt'
import { WindowChart } from '../components/WindowChart'
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
        </>
      )}
    </section>
  )
}

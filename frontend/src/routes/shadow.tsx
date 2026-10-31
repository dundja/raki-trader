import { Link, createFileRoute } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { useState, type ReactNode } from 'react'

import { getJson, qk } from '../api/client'
import type { Calibration } from '../api/generated/Calibration'
import type { ShadowFillRow } from '../api/generated/ShadowFillRow'
import type { ShadowSummary } from '../api/generated/ShadowSummary'
import { fmtCents, fmtInt, fmtLocal, fmtNum, fmtPct, fmtUsd, fmtUtc } from '../components/fmt'
import { fillLabel, fillPrice, skipLabel } from '../components/shadow'

export const Route = createFileRoute('/shadow')({ component: ShadowPage })

const REFETCH_MS = 15_000
/** Checkpoints offered for calibration (seconds before the window end). */
const CHECKPOINTS = [120, 60, 45, 30, 20, 10] as const
const ASSETS = [null, 'btc', 'eth', 'sol'] as const

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="card">
      <h2>{title}</h2>
      {children}
    </section>
  )
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="field">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}

function ratio(num: number, den: number): number | null {
  return den > 0 ? num / den : null
}

export function SummaryView({ s }: { s: ShadowSummary }) {
  const fills = s.filled + s.partial
  const staked = Number(s.staked)
  const returnPerDollar = staked > 0 ? Number(s.pnl) / staked : null
  return (
    <>
      <dl className="fields" data-testid="shadow-summary">
        <Field label="Evaluations">{fmtInt(s.evaluations)}</Field>
        <Field label="BTC windows evaluated">{fmtInt(s.windows_evaluated)}</Field>
        <Field label="Windows with an entry">{fmtInt(s.windows_entered)}</Field>
        <Field label="Entries">{fmtInt(s.entries)}</Field>
        <Field label="Filled / partly / no fill">
          {fmtInt(s.filled)} / {fmtInt(s.partial)} / {fmtInt(s.no_fill)}
        </Field>
        <Field label="No-fill rate">{fmtPct(ratio(s.no_fill, s.entries))}</Field>
        <Field label="Resolved fills">{fmtInt(s.resolved)}</Field>
        <Field label="Wins / losses">
          {fmtInt(s.wins)} / {fmtInt(s.losses)} ({fmtPct(ratio(s.wins, s.resolved))})
        </Field>
        <Field label="Staked (fees incl.)">${fmtNum(s.staked)}</Field>
        <Field label="PnL">
          <span className={Number(s.pnl) < 0 ? 'bad' : 'ok'} data-testid="shadow-pnl">
            {fmtUsd(s.pnl)} USD
          </span>
        </Field>
        <Field label="Return per $1">{fmtCents(returnPerDollar)}</Field>
        <Field label="Mean predicted edge (fills)">{fmtCents(s.mean_predicted_edge)}</Field>
        <Field label="Fills not yet resolved">{fmtInt(fills - s.resolved)}</Field>
      </dl>
      {s.skip_reasons.length > 0 && (
        <>
          <h3>Why it skipped</h3>
          <table className="table" data-testid="skip-reasons">
            <tbody>
              {s.skip_reasons.map((r) => (
                <tr key={r.reason}>
                  <td>{skipLabel(r.reason)}</td>
                  <td className="num">{fmtInt(r.count)}</td>
                  <td className="num">{fmtPct(ratio(r.count, s.evaluations))}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
      <p className="muted">
        Parameter sets: {s.params_hashes.length > 0 ? s.params_hashes.join(', ') : '—'}
      </p>
    </>
  )
}

export function CalibrationView({ c }: { c: Calibration }) {
  return (
    <>
      <p data-testid="calibration-n">
        {fmtInt(c.n)} resolved windows · Brier score{' '}
        {c.brier === null ? '—' : c.brier.toFixed(4)} (0.25 is a coin flip; lower is better)
      </p>
      <table className="table" data-testid="calibration">
        <thead>
          <tr>
            <th>Favourite's predicted probability</th>
            <th className="num">Windows</th>
            <th className="num">Mean predicted</th>
            <th className="num">Realized</th>
            <th className="num">95% interval</th>
          </tr>
        </thead>
        <tbody>
          {c.buckets.map((b) => {
            // Over-confident: the whole interval lies below what the model predicted.
            const over =
              b.ci_hi !== null && b.mean_predicted !== null && b.ci_hi < b.mean_predicted
            return (
              <tr key={b.label} data-state={over ? 'stale' : undefined}>
                <td>{b.label}</td>
                <td className="num">{fmtInt(b.n)}</td>
                <td className="num">{fmtPct(b.mean_predicted)}</td>
                <td className={over ? 'num bad' : 'num'}>{fmtPct(b.realized)}</td>
                <td className="num">
                  {b.ci_lo === null || b.ci_hi === null
                    ? '—'
                    : `${fmtPct(b.ci_lo)} – ${fmtPct(b.ci_hi)}`}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </>
  )
}

export function FillsTable({ fills }: { fills: ShadowFillRow[] }) {
  if (fills.length === 0) return <p className="muted">No simulated fills yet.</p>
  return (
    <div className="table-scroll">
      <table className="table" data-testid="shadow-fills">
        <thead>
          <tr>
            <th>Time</th>
            <th>Window</th>
            <th>Side</th>
            <th>Result</th>
            <th className="num">Shares</th>
            <th className="num">Avg price</th>
            <th className="num">Fee</th>
            <th>Winner</th>
            <th className="num">PnL</th>
          </tr>
        </thead>
        <tbody>
          {fills.map((f) => (
            <tr key={`${f.session_id}-${f.seq}`}>
              <td className="nowrap" title={fmtUtc(f.filled_ms)}>
                {fmtLocal(f.filled_ms)}
              </td>
              <td>
                <Link to="/windows/$slug" params={{ slug: f.slug }}>
                  {f.slug}
                </Link>
              </td>
              <td>{f.side}</td>
              <td>{fillLabel(f)}</td>
              <td className="num">{fmtNum(f.shares)}</td>
              <td className="num">{fillPrice(f)}</td>
              <td className="num">{fmtNum(f.fee, 4)}</td>
              <td>{f.winner ?? '—'}</td>
              <td className="num">{f.pnl === null ? '—' : fmtUsd(f.pnl)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function ShadowPage() {
  const [checkpoint, setCheckpoint] = useState<number>(30)
  const [asset, setAsset] = useState<string | null>('btc')
  const summary = useQuery({
    queryKey: qk.shadowSummary,
    queryFn: () => getJson<ShadowSummary>('/api/shadow/summary'),
    refetchInterval: REFETCH_MS,
  })
  const calibration = useQuery({
    queryKey: qk.calibration(checkpoint, asset),
    queryFn: () =>
      getJson<Calibration>(
        `/api/shadow/calibration?checkpoint=${checkpoint}${asset ? `&asset=${asset}` : ''}`,
      ),
    refetchInterval: REFETCH_MS,
  })
  const fills = useQuery({
    queryKey: qk.shadowFills,
    queryFn: () => getJson<ShadowFillRow[]>('/api/shadow/fills?limit=50'),
    refetchInterval: REFETCH_MS,
  })

  return (
    <div className="health">
      <p className="banner banner-warn" role="note">
        Shadow mode: the model and strategy v0 run on live data with simulated fills. No orders are
        placed, and none of these numbers count toward the M2 or go/no-go gates.
      </p>
      <Section title="Strategy v0 (BTC, $5 per window)">
        {summary.isError && <p className="dialog-error">{summary.error.message}</p>}
        {summary.data ? <SummaryView s={summary.data} /> : <p className="muted">Loading…</p>}
      </Section>
      <Section title="Model calibration">
        <div className="buttons">
          <label>
            Checkpoint{' '}
            <select
              value={checkpoint}
              onChange={(e) => setCheckpoint(Number(e.target.value))}
            >
              {CHECKPOINTS.map((c) => (
                <option key={c} value={c}>
                  T−{c} s
                </option>
              ))}
            </select>
          </label>
          <label>
            Asset{' '}
            <select
              value={asset ?? ''}
              onChange={(e) => setAsset(e.target.value === '' ? null : e.target.value)}
            >
              {ASSETS.map((a) => (
                <option key={a ?? 'all'} value={a ?? ''}>
                  {a === null ? 'all' : a.toUpperCase()}
                </option>
              ))}
            </select>
          </label>
        </div>
        {calibration.isError && <p className="dialog-error">{calibration.error.message}</p>}
        {calibration.data ? (
          <CalibrationView c={calibration.data} />
        ) : (
          <p className="muted">Loading…</p>
        )}
      </Section>
      <Section title="Recent simulated fills">
        {fills.isError && <p className="dialog-error">{fills.error.message}</p>}
        {fills.data ? <FillsTable fills={fills.data} /> : <p className="muted">Loading…</p>}
      </Section>
    </div>
  )
}

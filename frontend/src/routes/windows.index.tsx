import { Link, createFileRoute } from '@tanstack/react-router'
import { useInfiniteQuery } from '@tanstack/react-query'

import { getJson, qk } from '../api/client'
import type { Asset } from '../api/generated/Asset'
import type { WindowRecord } from '../api/generated/WindowRecord'
import type { WindowStatus } from '../api/generated/WindowStatus'
import { fmtLocal, fmtPrice, fmtUtc } from '../components/fmt'

const ASSETS: readonly Asset[] = ['btc', 'eth', 'sol']
const PAGE_SIZE = 50
const WINDOW_SECS = 300

type Search = { asset?: Asset }

export const Route = createFileRoute('/windows/')({
  validateSearch: (raw: Record<string, unknown>): Search => ({
    asset: ASSETS.find((a) => a === raw.asset),
  }),
  component: WindowsRoute,
})

function WindowsRoute() {
  const { asset } = Route.useSearch()
  const navigate = Route.useNavigate()
  return (
    <WindowsPage
      asset={asset ?? null}
      onAssetChange={(a) => void navigate({ search: { asset: a ?? undefined } })}
    />
  )
}

const TERMINAL: readonly WindowStatus[] = ['complete', 'timed_out']

/** TWAP-vs-Gamma comparison: ✓ match, ✗ differs, … still waiting for it, — will never be compared. */
export function MatchBadge({
  match,
  status,
}: {
  match: boolean | null
  status: WindowStatus
}) {
  if (match === true) {
    return (
      <span className="badge badge-ok" aria-label="TWAP matches Gamma">
        ✓
      </span>
    )
  }
  if (match === false) {
    return (
      <span className="badge badge-bad" aria-label="TWAP differs from Gamma">
        ✗
      </span>
    )
  }
  if (TERMINAL.includes(status)) {
    return (
      <span className="badge badge-idle" aria-label="No comparison available">
        —
      </span>
    )
  }
  return (
    <span className="badge badge-warn" aria-label="Match pending">
      …
    </span>
  )
}

const STATUS_CLASS: Record<WindowStatus, string> = {
  upcoming: 'badge-idle',
  live: 'badge-warn',
  ended: 'badge-warn',
  resolved: 'badge-ok',
  complete: 'badge-ok',
  timed_out: 'badge-bad',
}

export function StatusBadge({ status }: { status: WindowStatus }) {
  return <span className={`badge ${STATUS_CLASS[status]}`}>{status}</span>
}

/** Seconds from the window end until we saw it resolved. */
export function resolutionLatency(w: WindowRecord): string {
  if (w.resolution_seen_ms === null) return '—'
  const secs = w.resolution_seen_ms / 1000 - w.end_ts
  return Number.isFinite(secs) ? `${secs.toFixed(1)} s` : '—'
}

export function coveragePct(w: WindowRecord): string {
  return `${Math.round((w.coverage_secs / WINDOW_SECS) * 100)}%`
}

function listPath(asset: Asset | null, before: number | null): string {
  const q = new URLSearchParams()
  if (asset !== null) q.set('asset', asset)
  q.set('limit', String(PAGE_SIZE))
  if (before !== null) q.set('before', String(before))
  return `/api/windows?${q.toString()}`
}

type Props = {
  asset: Asset | null
  onAssetChange: (asset: Asset | null) => void
}

export function WindowsPage({ asset, onAssetChange }: Props) {
  const query = useInfiniteQuery({
    queryKey: qk.windows(asset, null),
    queryFn: ({ pageParam }) => getJson<WindowRecord[]>(listPath(asset, pageParam)),
    initialPageParam: null as number | null,
    // A short page is the last one. Otherwise continue strictly before the oldest row we have.
    getNextPageParam: (last) =>
      last.length < PAGE_SIZE ? undefined : last[last.length - 1].start_ts,
    staleTime: 15_000,
    refetchOnWindowFocus: false,
  })

  const seen = new Set<string>()
  const rows: WindowRecord[] = []
  for (const page of query.data?.pages ?? []) {
    for (const w of page) {
      if (seen.has(w.slug)) continue
      seen.add(w.slug)
      rows.push(w)
    }
  }

  return (
    <section>
      <div className="toolbar">
        <h1>Windows</h1>
        <label>
          Asset{' '}
          <select
            value={asset ?? 'all'}
            onChange={(e) => {
              const v = e.target.value
              onAssetChange(ASSETS.find((a) => a === v) ?? null)
            }}
          >
            <option value="all">all</option>
            {ASSETS.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
        </label>
      </div>

      {query.isPending && <p className="muted">Loading windows…</p>}
      {query.isError && (
        <p className="dialog-error" role="alert">
          Could not load windows: {query.error.message}
        </p>
      )}
      {query.isSuccess && rows.length === 0 && (
        <p className="muted">No windows recorded yet.</p>
      )}

      {rows.length > 0 && (
        <div className="table-scroll">
          <table className="table">
            <thead>
              <tr>
                <th>Start</th>
                <th>Asset</th>
                <th>Status</th>
                <th className="num">PTB (ours)</th>
                <th className="num">Final (ours)</th>
                <th>Outcome (ours)</th>
                <th className="num">Final (Gamma)</th>
                <th>Match</th>
                <th>Winner</th>
                <th className="num">Resolution latency</th>
                <th className="num">Coverage</th>
                <th>TWAP source</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((w) => (
                <tr key={w.slug} data-testid={`window-row-${w.slug}`}>
                  <td className="nowrap" title={fmtUtc(w.start_ts * 1000)}>
                    <Link to="/windows/$slug" params={{ slug: w.slug }}>
                      {fmtLocal(w.start_ts * 1000)}
                    </Link>
                  </td>
                  <td>{w.asset}</td>
                  <td>
                    <StatusBadge status={w.status} />
                  </td>
                  <td className="num">{fmtPrice(w.price_to_beat_ours)}</td>
                  <td className="num">{fmtPrice(w.final_twap_ours)}</td>
                  <td>{w.outcome_ours ?? '—'}</td>
                  <td className="num">{fmtPrice(w.final_price_gamma)}</td>
                  <td>
                    <MatchBadge match={w.twap_match} status={w.status} />
                  </td>
                  <td>{w.winner ?? '—'}</td>
                  <td className="num">{resolutionLatency(w)}</td>
                  <td className="num">{coveragePct(w)}</td>
                  <td>{w.twap_source ?? '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {query.hasNextPage && (
        <div className="buttons">
          <button
            type="button"
            onClick={() => void query.fetchNextPage()}
            disabled={query.isFetchingNextPage}
          >
            {query.isFetchingNextPage ? 'Loading…' : 'Load more'}
          </button>
        </div>
      )}
    </section>
  )
}

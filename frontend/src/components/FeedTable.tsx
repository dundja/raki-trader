import type { FeedStatusDto } from '../api/generated/FeedStatusDto'
import { fmtAgo, fmtInt, fmtUtc } from './fmt'

type RowState = 'ok' | 'stale' | 'disabled'

function rowState(f: FeedStatusDto): RowState {
  if (!f.enabled) return 'disabled'
  return f.stale ? 'stale' : 'ok'
}

function disabledNote(f: FeedStatusDto): string {
  return f.source === 'polybolt' ? 'disabled (no credentials)' : 'disabled'
}

/** One row per data source. Stale rows are tinted red, disabled rows are greyed. */
export function FeedTable({ feeds, now }: { feeds: FeedStatusDto[]; now: number }) {
  return (
    <table className="table feed-table">
      <thead>
        <tr>
          <th>Source</th>
          <th>Enabled</th>
          <th>Connected</th>
          <th>Last message</th>
          <th className="num">Msgs/s</th>
          <th className="num">Reconnects</th>
          <th>Last error</th>
          <th>Flag</th>
        </tr>
      </thead>
      <tbody>
        {feeds.map((f) => {
          const state = rowState(f)
          return (
            <tr key={f.source} data-testid={`feed-${f.source}`} data-state={state}>
              <th scope="row">{f.source}</th>
              <td>{f.enabled ? 'yes' : 'no'}</td>
              <td>{f.connected ? 'yes' : 'no'}</td>
              <td title={f.last_msg_ms === null ? undefined : fmtUtc(f.last_msg_ms)}>
                {fmtAgo(f.last_msg_ms, now)}
              </td>
              <td className="num">{f.msgs_per_sec.toFixed(1)}</td>
              <td className="num">{fmtInt(f.reconnects)}</td>
              <td className="error-cell">{f.last_error ?? '—'}</td>
              <td>
                {state === 'stale' && <span className="badge badge-bad">STALE</span>}
                {state === 'disabled' && <span className="muted">{disabledNote(f)}</span>}
              </td>
            </tr>
          )
        })}
      </tbody>
    </table>
  )
}

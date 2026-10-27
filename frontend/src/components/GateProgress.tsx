import type { RecordingStats } from '../api/generated/RecordingStats'
import { fmtInt } from './fmt'

/** Progress toward the M1 gate, from /api/stats. `stats` is null until the first response. */
export function GateProgress({ stats }: { stats: RecordingStats | null }) {
  if (stats === null) {
    return <p className="muted">Stats unavailable.</p>
  }
  const target = stats.gate_target_windows
  const pct = target > 0 ? Math.min(100, (stats.windows_with_twap / target) * 100) : 0
  const label = `M1 gate: ${stats.windows_with_twap} / ${target} windows with TWAP`
  return (
    <div className="gate-progress">
      <p className="gate-label">{label}</p>
      <div
        className="progress"
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={target}
        aria-valuenow={stats.windows_with_twap}
      >
        <div className="progress-fill" style={{ width: `${pct}%` }} />
      </div>
      <p data-testid="twap-tally">
        TWAP vs Gamma: {fmtInt(stats.twap_match)} match · {fmtInt(stats.twap_mismatch)} mismatch ·{' '}
        {fmtInt(stats.twap_pending)} pending
      </p>
      <p data-testid="recorded">
        Recorded {(stats.recorded_secs / 3600).toFixed(1)} h in {stats.sessions}{' '}
        {stats.sessions === 1 ? 'session' : 'sessions'} · {fmtInt(stats.windows_total)} windows total
      </p>
    </div>
  )
}

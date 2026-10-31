import type { RecordingStats } from '../api/generated/RecordingStats'
import { fmtInt } from './fmt'

function hours(secs: number): string {
  return `${(secs / 3600).toFixed(1)} h`
}

/**
 * Progress toward the M1 gate, from /api/stats. `stats` is null until the first response.
 *
 * The gate (CLAUDE.md §5) counts only BTC windows whose final TWAP came from PolyBolt, and checks
 * that TWAP against Gamma's finalPrice. Windows of other assets or recorded with the legacy RTDS
 * feed are shown separately, as information: they never move the bar.
 */
export function GateProgress({ stats }: { stats: RecordingStats | null }) {
  if (stats === null) {
    return <p className="muted">Stats unavailable.</p>
  }
  const target = stats.gate_target_windows
  const pct = target > 0 ? Math.min(100, (stats.gate_windows / target) * 100) : 0
  const label = `M1 gate: ${fmtInt(stats.gate_windows)} / ${fmtInt(target)} BTC windows with a PolyBolt TWAP`
  return (
    <div className="gate-progress">
      <p className="gate-label">{label}</p>
      <div
        className="progress"
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={target}
        aria-valuenow={stats.gate_windows}
      >
        <div className="progress-fill" style={{ width: `${pct}%` }} />
      </div>
      <p data-testid="twap-tally">
        Gate TWAP vs Gamma: {fmtInt(stats.gate_twap_match)} match ·{' '}
        {fmtInt(stats.gate_twap_mismatch)} mismatch · {fmtInt(stats.gate_twap_awaiting)} awaiting ·{' '}
        {fmtInt(stats.gate_twap_unverified)} unverified ·{' '}
        <span className={stats.gate_twap_missed_end > 0 ? 'bad' : undefined}>
          {fmtInt(stats.gate_twap_missed_end)} recorded through the end with no final TWAP point
        </span>
      </p>
      <p className="muted" data-testid="all-tally">
        All assets and sources (not the gate): {fmtInt(stats.windows_with_twap)} windows with TWAP ·{' '}
        {fmtInt(stats.twap_match)} match · {fmtInt(stats.twap_mismatch)} mismatch ·{' '}
        {fmtInt(stats.twap_awaiting)} awaiting · {fmtInt(stats.twap_unverified)} unverified ·{' '}
        {fmtInt(stats.twap_missed_end)} no final point
      </p>
      <p data-testid="recorded">
        Recorded {hours(stats.recorded_secs)} awake in {fmtInt(stats.sessions)}{' '}
        {stats.sessions === 1 ? 'session' : 'sessions'}
        {stats.slept_secs > 0 && ` (${hours(stats.slept_secs)} asleep not counted)`} ·{' '}
        {fmtInt(stats.windows_total)} windows total
      </p>
    </div>
  )
}

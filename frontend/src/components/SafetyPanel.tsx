import type { SafetyChecks } from '../api/generated/SafetyChecks'

const CHECKS: ReadonlyArray<{ key: keyof Omit<SafetyChecks, 'all_ok' | 'reasons'>; label: string }> = [
  { key: 'feeds_fresh', label: 'Feeds fresh' },
  { key: 'clock_ok', label: 'Clock offset within limit' },
  { key: 'geoblock_ok', label: 'Not geoblocked' },
  { key: 'recorder_ok', label: 'Recorder healthy' },
  { key: 'store_ok', label: 'Database writes succeeding' },
]

/**
 * Hard safety checks the backend enforces before a resume, each with a mark and the reasons.
 * `stale`: the live data stopped arriving, so these results are old and say nothing about now.
 */
export function SafetyPanel({ safety, stale = false }: { safety: SafetyChecks; stale?: boolean }) {
  const [cls, text] = stale
    ? ['badge badge-warn', 'Unknown: no live data']
    : safety.all_ok
      ? ['badge badge-ok', 'All checks pass']
      : ['badge badge-bad', 'Checks failing']
  return (
    <div className="safety-panel" data-stale={stale}>
      <p className={cls} data-testid="safety-verdict">
        {text}
      </p>
      <ul className="checks">
        {CHECKS.map(({ key, label }) => (
          <li key={key} data-testid={`check-${key}`} data-ok={safety[key]}>
            <span className={safety[key] ? 'mark-ok' : 'mark-bad'} aria-hidden="true">
              {safety[key] ? '✓' : '✗'}
            </span>
            <span className="sr-only">{safety[key] ? 'pass' : 'fail'}: </span>
            {label}
          </li>
        ))}
      </ul>
      {safety.reasons.length > 0 && (
        <ul className="reasons" aria-label="Reasons">
          {safety.reasons.map((r) => (
            <li key={r}>{r}</li>
          ))}
        </ul>
      )}
    </div>
  )
}

import type { SafetyChecks } from '../api/generated/SafetyChecks'

const CHECKS: ReadonlyArray<{ key: keyof Omit<SafetyChecks, 'all_ok' | 'reasons'>; label: string }> = [
  { key: 'feeds_fresh', label: 'Feeds fresh' },
  { key: 'clock_ok', label: 'Clock offset within limit' },
  { key: 'geoblock_ok', label: 'Not geoblocked' },
  { key: 'recorder_ok', label: 'Recorder healthy' },
]

/** Hard safety checks the backend enforces before a resume, each with a mark and the reasons. */
export function SafetyPanel({ safety }: { safety: SafetyChecks }) {
  return (
    <div className="safety-panel">
      <p className={safety.all_ok ? 'badge badge-ok' : 'badge badge-bad'}>
        {safety.all_ok ? 'All checks pass' : 'Checks failing'}
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

const DASH = '—'

function toNumber(v: number | string | null | undefined): number | null {
  if (v === null || v === undefined || v === '') return null
  const n = typeof v === 'number' ? v : Number(v)
  return Number.isFinite(n) ? n : null
}

/** Display-only formatting of a price (decimals arrive as strings). null/invalid → "—". */
export function fmtPrice(
  v: number | string | null | undefined,
  digits = 2,
): string {
  const n = toNumber(v)
  if (n === null) return DASH
  return n.toLocaleString('en-US', {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  })
}

/** A 0..1 probability or share price as cents: "0.95" → "95.0¢". */
export function fmtProb(v: string | null | undefined): string {
  const n = toNumber(v)
  if (n === null) return DASH
  return `${(n * 100).toFixed(1)}¢`
}

/** Seconds left in a window as m:ss. At or past the end (or invalid) never shows a negative timer. */
export function fmtCountdown(secsLeft: number): string {
  if (!Number.isFinite(secsLeft)) return DASH
  if (secsLeft <= 0) return 'ended'
  const total = Math.floor(secsLeft)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${String(s).padStart(2, '0')}`
}

/** Age of an epoch-ms timestamp relative to `now` (epoch ms). */
export function fmtAgo(ms: number | null | undefined, now: number): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return DASH
  const secs = Math.max(0, Math.floor((now - ms) / 1000))
  if (secs < 60) return `${secs}s ago`
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  return `${Math.floor(hours / 24)}d ago`
}

/** Epoch ms as an ISO-like UTC string, for titles and tooltips. */
export function fmtUtc(ms: number): string {
  if (!Number.isFinite(ms)) return DASH
  return `${new Date(ms).toISOString().slice(0, 19).replace('T', ' ')} UTC`
}

/** A number with an explicit sign ("+40.00", "-3.20"). Display only. null/invalid → "—". */
export function fmtSigned(v: number | null | undefined, digits = 2): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return DASH
  return v.toLocaleString('en-US', {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
    signDisplay: 'always',
  })
}

/** A plain count with thousands separators. null/invalid → "—". */
export function fmtInt(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return DASH
  return v.toLocaleString('en-US', { maximumFractionDigits: 0 })
}

/** Bytes as megabytes (1 MB = 1e6 bytes), one decimal. */
export function fmtMb(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return DASH
  return `${(bytes / 1e6).toLocaleString('en-US', {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  })} MB`
}

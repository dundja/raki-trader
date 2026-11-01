import type { LiveShadow } from '../api/generated/LiveShadow'
import type { ShadowFillRow } from '../api/generated/ShadowFillRow'
import { fmtCents, fmtNum, fmtProb } from './fmt'

/** Why strategy v0 skipped, in words (`reason` values of ShadowEvalRow). */
export const SKIP_REASON: Record<string, string> = {
  unsafe: 'a safety check failing',
  not_running: 'bot paused (older rows)',
  no_prediction: 'no prediction',
  market_config: 'market config missing or unexpected',
  no_book: 'no order book',
  no_favourite: 'no side at 92¢ or more',
  no_liquidity: 'not enough asks at 97¢ or less',
  edge: 'edge below 1¢',
}

export function skipLabel(reason: string | null): string {
  if (reason === null) return '—'
  return SKIP_REASON[reason] ?? reason
}

const FILL_LABEL: Record<ShadowFillRow['status'], string> = {
  filled: 'Filled',
  partial: 'Partly filled',
  no_fill: 'No fill',
}

export function fillLabel(f: ShadowFillRow): string {
  return FILL_LABEL[f.status]
}

/** Average price per share of a fill, as cents. */
export function fillPrice(f: ShadowFillRow): string {
  const shares = Number(f.shares)
  return shares > 0 ? fmtProb(String(Number(f.cost) / shares)) : '—'
}

/** One line for the live card: what v0 did in this window so far (simulated, never an order). */
export function shadowText(s: LiveShadow): string {
  const fill = s.fill
  if (fill && fill.status !== 'no_fill') {
    return `${fillLabel(fill)}: ${fill.side} ${fmtNum(fill.shares)} sh @ ${fillPrice(fill)} (simulated)`
  }
  if (s.pending) return 'Entering… (simulated fill pending)'
  const e = s.last_eval
  if (e === null) return 'Waits for T−45 s'
  if (e.action === 'enter') return `Enter ${e.side ?? ''}, edge ${fmtCents(e.edge)}`
  const edge = e.edge === null ? '' : ` (edge ${fmtCents(e.edge)})`
  const noFill = fill?.status === 'no_fill' ? 'last try: no fill · ' : ''
  return `${noFill}Skip: ${skipLabel(e.reason)}${edge}`
}

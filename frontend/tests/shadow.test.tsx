import { cleanup, render, screen, within } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import type { Calibration } from '../src/api/generated/Calibration'
import type { LiveShadow } from '../src/api/generated/LiveShadow'
import type { ShadowEvalRow } from '../src/api/generated/ShadowEvalRow'
import type { ShadowFillRow } from '../src/api/generated/ShadowFillRow'
import type { ShadowSummary } from '../src/api/generated/ShadowSummary'
import { fmtCents, fmtPct, fmtUsd } from '../src/components/fmt'
import { shadowText, skipLabel } from '../src/components/shadow'
import { CalibrationView, SummaryView } from '../src/routes/shadow'

afterEach(cleanup)

function evalRow(over: Partial<ShadowEvalRow> = {}): ShadowEvalRow {
  return {
    session_id: 's',
    seq: 1,
    strategy_id: 'v0',
    params_hash: 'h',
    slug: 'btc-updown-5m-1793000100',
    ts_ms: 0,
    secs_left: 45,
    side: 'Up',
    model_prob: 0.98,
    market_prob: '0.935',
    best_ask: '0.94',
    walk_shares: '5.3',
    walk_avg_price: '0.94',
    walk_fee: '0.021',
    edge: '0.036052',
    action: 'enter',
    reason: null,
    control_running: true,
    ...over,
  }
}

function fillRow(over: Partial<ShadowFillRow> = {}): ShadowFillRow {
  return {
    session_id: 's',
    seq: 1,
    slug: 'btc-updown-5m-1793000100',
    side: 'Up',
    decided_ms: 0,
    due_ms: 250,
    filled_ms: 300,
    limit_price: '0.94',
    shares: '5.3191489',
    cost: '5',
    fee: '0.021',
    status: 'filled',
    winner: null,
    pnl: null,
    ...over,
  }
}

describe('formatting', () => {
  it('percent, signed dollars and cents', () => {
    expect(fmtPct(0.97314)).toBe('97.3%')
    expect(fmtPct(null)).toBe('—')
    expect(fmtUsd('-4.72')).toBe('-4.72')
    expect(fmtUsd('0.28')).toBe('+0.28')
    expect(fmtCents('0.036052')).toBe('+3.6¢')
    expect(fmtCents(null)).toBe('—')
  })
})

describe('shadowText', () => {
  const base: LiveShadow = { last_eval: null, fill: null, pending: false }

  it('describes each v0 state of a window', () => {
    expect(shadowText(base)).toBe('Waits for T−45 s')
    expect(shadowText({ ...base, pending: true })).toMatch(/^Entering/)
    expect(shadowText({ ...base, last_eval: evalRow() })).toBe('Enter Up, edge +3.6¢')
    expect(
      shadowText({
        ...base,
        last_eval: evalRow({ action: 'skip', reason: 'edge', edge: '0.006' }),
      }),
    ).toBe('Skip: edge below 1¢ (edge +0.6¢)')
    expect(shadowText({ ...base, fill: fillRow() })).toBe(
      'Filled: Up 5.32 sh @ 94.0¢ (simulated)',
    )
    expect(
      shadowText({
        ...base,
        fill: fillRow({ status: 'no_fill' }),
        last_eval: evalRow({ action: 'skip', reason: 'no_liquidity', edge: null }),
      }),
    ).toBe('last try: no fill · Skip: not enough asks at 97¢ or less')
  })

  it('unknown skip reasons are shown as they are', () => {
    expect(skipLabel('something_new')).toBe('something_new')
    expect(skipLabel(null)).toBe('—')
  })
})

describe('Shadow page views', () => {
  it('summary shows fills, PnL and skip reasons', () => {
    const s: ShadowSummary = {
      evaluations: 160,
      evaluations_while_paused: 20,
      windows_evaluated: 10,
      windows_entered: 3,
      entries: 4,
      filled: 2,
      partial: 1,
      no_fill: 1,
      resolved: 2,
      wins: 1,
      losses: 1,
      staked: '9.44',
      pnl: '-4.44',
      mean_predicted_edge: 0.036,
      skip_reasons: [
        { reason: 'no_favourite', count: 120 },
        { reason: 'edge', count: 36 },
      ],
      params_hashes: ['954ed69967b57fe0'],
    }
    render(<SummaryView s={s} />)
    const summary = screen.getByTestId('shadow-summary')
    expect(within(summary).getByText('No-fill rate').closest('div')).toHaveTextContent('25.0%')
    expect(screen.getByTestId('shadow-pnl')).toHaveTextContent('-4.44 USD')
    expect(screen.getByTestId('shadow-pnl')).toHaveClass('bad')
    expect(within(summary).getByText('Return per $1').closest('div')).toHaveTextContent('-47.0¢')
    const reasons = screen.getByTestId('skip-reasons')
    expect(reasons).toHaveTextContent('no side at 92¢ or more')
    expect(reasons).toHaveTextContent('75.0%')
    expect(screen.getByText(/954ed69967b57fe0/)).toBeInTheDocument()
    expect(within(summary).getByText('…while the bot was paused').closest('div')).toHaveTextContent('20')
  })

  it('calibration flags a bucket whose interval lies below the prediction', () => {
    const c: Calibration = {
      checkpoint_s: 30,
      asset: 'btc',
      n: 40,
      brier: 0.0612,
      buckets: [
        {
          label: '0.95–0.97',
          lo: 0.95,
          hi: 0.97,
          n: 30,
          mean_predicted: 0.96,
          realized: 0.7,
          ci_lo: 0.52,
          ci_hi: 0.83,
        },
        {
          label: '0.97–0.99',
          lo: 0.97,
          hi: 0.99,
          n: 10,
          mean_predicted: 0.98,
          realized: 1,
          ci_lo: 0.72,
          ci_hi: 1,
        },
        {
          label: '≥ 0.99',
          lo: 0.99,
          hi: 1,
          n: 0,
          mean_predicted: null,
          realized: null,
          ci_lo: null,
          ci_hi: null,
        },
      ],
    }
    render(<CalibrationView c={c} />)
    expect(screen.getByTestId('calibration-n')).toHaveTextContent('40 resolved windows')
    expect(screen.getByTestId('calibration-n')).toHaveTextContent('0.0612')
    const rows = within(screen.getByTestId('calibration')).getAllByRole('row')
    expect(rows[1]).toHaveAttribute('data-state', 'stale')
    expect(rows[1]).toHaveTextContent('52.0% – 83.0%')
    expect(rows[2]).not.toHaveAttribute('data-state')
    expect(rows[3]).toHaveTextContent('—')
  })
})

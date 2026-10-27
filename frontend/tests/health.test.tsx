import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../src/api/client', async (orig) => ({
  ...(await orig<typeof import('../src/api/client')>()),
  postControl: vi.fn(),
}))

import { postControl } from '../src/api/client'
import { ControlPanel } from '../src/components/ControlPanel'
import { FeedTable } from '../src/components/FeedTable'
import { GateProgress } from '../src/components/GateProgress'
import { SafetyPanel } from '../src/components/SafetyPanel'
import type { ControlState } from '../src/api/generated/ControlState'
import type { FeedStatusDto } from '../src/api/generated/FeedStatusDto'

const NOW = 1_793_000_000_000

function feed(over: Partial<FeedStatusDto> = {}): FeedStatusDto {
  return {
    source: 'binance',
    enabled: true,
    connected: true,
    last_msg_ms: NOW - 1000,
    msgs_total: 1000,
    msgs_per_sec: 12.3,
    reconnects: 0,
    last_error: null,
    stale: false,
    ...over,
  }
}

function control(over: Partial<ControlState> = {}): ControlState {
  return {
    mode: 'paused',
    stop_kind: 'safety',
    reason: 'clock offset 63.1 ms > 50 ms',
    needs_ack: false,
    since_ms: NOW - 60_000,
    ...over,
  }
}

const mockedPost = vi.mocked(postControl)

beforeEach(() => {
  mockedPost.mockReset()
})
afterEach(cleanup)

describe('FeedTable', () => {
  it('feed table marks stale', () => {
    render(
      <FeedTable
        now={NOW}
        feeds={[
          feed({ source: 'binance' }),
          feed({ source: 'coinbase', stale: true, last_error: 'timeout' }),
          feed({
            source: 'polybolt',
            enabled: false,
            connected: false,
            last_msg_ms: null,
          }),
        ]}
      />,
    )
    expect(screen.getByTestId('feed-binance')).toHaveAttribute('data-state', 'ok')
    const stale = screen.getByTestId('feed-coinbase')
    expect(stale).toHaveAttribute('data-state', 'stale')
    expect(stale).toHaveTextContent('STALE')
    expect(stale).toHaveTextContent('timeout')
    const disabled = screen.getByTestId('feed-polybolt')
    expect(disabled).toHaveAttribute('data-state', 'disabled')
    expect(disabled).toHaveTextContent('disabled (no credentials)')
    expect(screen.getByTestId('feed-binance')).toHaveTextContent('1s ago')
    expect(screen.getByTestId('feed-binance')).toHaveTextContent('12.3')
  })
})

describe('SafetyPanel', () => {
  it('safety panel lists reasons', () => {
    render(
      <SafetyPanel
        safety={{
          feeds_fresh: true,
          clock_ok: false,
          geoblock_ok: true,
          recorder_ok: true,
          all_ok: false,
          reasons: ['clock offset 63.1 ms > 50 ms'],
        }}
      />,
    )
    expect(screen.getByTestId('check-clock_ok')).toHaveTextContent('✗')
    expect(screen.getByTestId('check-feeds_fresh')).toHaveTextContent('✓')
    expect(screen.getByRole('list', { name: 'Reasons' })).toHaveTextContent(
      'clock offset 63.1 ms > 50 ms',
    )
  })
})

describe('GateProgress', () => {
  it('gate progress shows N / 500', () => {
    render(
      <GateProgress
        stats={{
          sessions: 3,
          recorded_secs: 45_367,
          windows_total: 330,
          windows_with_twap: 181,
          twap_match: 167,
          twap_mismatch: 0,
          twap_pending: 14,
          gate_target_windows: 500,
        }}
      />,
    )
    expect(
      screen.getByText('M1 gate: 181 / 500 windows with TWAP'),
    ).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '181')
    expect(screen.getByTestId('twap-tally')).toHaveTextContent(
      '167 match · 0 mismatch · 14 pending',
    )
    expect(screen.getByTestId('recorded')).toHaveTextContent('12.6 h')
    expect(screen.getByTestId('recorded')).toHaveTextContent('3 sessions')
  })

  it('gate progress without stats shows placeholders', () => {
    render(<GateProgress stats={null} />)
    expect(screen.getByText(/Stats unavailable/)).toBeInTheDocument()
  })
})

describe('ControlPanel', () => {
  it('resume requires ack checkbox when needs_ack', () => {
    render(
      <ControlPanel
        control={control({ stop_kind: 'loss_cap', reason: 'daily loss cap', needs_ack: true })}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Resume' }))
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('daily loss cap')
    const confirm = screen.getByRole('button', { name: 'Confirm' })
    expect(confirm).toBeDisabled()
    fireEvent.click(screen.getByLabelText('I acknowledge: daily loss cap'))
    expect(confirm).toBeEnabled()
  })

  it('resume without needs_ack has no checkbox and sends acknowledge=false', async () => {
    mockedPost.mockResolvedValue({ ok: true, state: control({ mode: 'running' }), error: null })
    render(<ControlPanel control={control()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Resume' }))
    expect(screen.queryByRole('checkbox')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    await waitFor(() =>
      expect(mockedPost).toHaveBeenCalledWith('resume', { reason: null, acknowledge: false }),
    )
  })

  it('resume sends acknowledge=true after ticking and shows the mode toast', async () => {
    mockedPost.mockResolvedValue({ ok: true, state: control({ mode: 'running' }), error: null })
    render(<ControlPanel control={control({ needs_ack: true })} />)
    fireEvent.click(screen.getByRole('button', { name: 'Resume' }))
    fireEvent.click(screen.getByRole('checkbox'))
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    await waitFor(() =>
      expect(mockedPost).toHaveBeenCalledWith('resume', { reason: null, acknowledge: true }),
    )
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(screen.getByRole('status')).toHaveTextContent('Mode: running')
  })

  it('resume shows backend error', async () => {
    const error = 'safety checks failing: clock offset 63.1 ms > 50 ms'
    mockedPost.mockResolvedValue({ ok: false, error, state: control() })
    render(<ControlPanel control={control()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Resume' }))
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent(error)
    // the dialog stays open and nothing is shown as changed
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(screen.queryByRole('status')).toBeNull()
    expect(screen.getByTestId('control-mode')).toHaveTextContent('PAUSED')
  })

  it('shows a transport failure inline too', async () => {
    mockedPost.mockRejectedValue(new Error('POST /api/control/resume failed: HTTP 500'))
    render(<ControlPanel control={control()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Resume' }))
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('HTTP 500')
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })

  it('pause sends the optional reason', async () => {
    mockedPost.mockResolvedValue({ ok: true, state: control({ mode: 'paused' }), error: null })
    render(<ControlPanel control={control({ mode: 'running', stop_kind: null, reason: null })} />)
    fireEvent.click(screen.getByRole('button', { name: 'Pause' }))
    fireEvent.change(screen.getByLabelText('Reason (optional)'), {
      target: { value: ' news event ' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    await waitFor(() =>
      expect(mockedPost).toHaveBeenCalledWith('pause', {
        reason: 'news event',
        acknowledge: null,
      }),
    )
  })

  it('kill requires typing KILL', async () => {
    mockedPost.mockResolvedValue({ ok: true, state: control({ mode: 'killed' }), error: null })
    render(<ControlPanel control={control({ mode: 'running', stop_kind: null, reason: null })} />)
    fireEvent.click(screen.getByRole('button', { name: 'Kill' }))
    const confirm = screen.getByRole('button', { name: 'Confirm' })
    expect(confirm).toBeDisabled()
    const input = screen.getByLabelText('Type KILL to confirm')
    fireEvent.change(input, { target: { value: 'kill' } })
    expect(confirm).toBeDisabled()
    fireEvent.change(input, { target: { value: 'KILL' } })
    expect(confirm).toBeEnabled()
    fireEvent.click(confirm)
    await waitFor(() =>
      expect(mockedPost).toHaveBeenCalledWith('kill', { reason: null, acknowledge: null }),
    )
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('Mode: killed'))
  })

  it('cancel closes the dialog without calling the backend', () => {
    render(<ControlPanel control={control()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Resume' }))
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(mockedPost).not.toHaveBeenCalled()
  })
})

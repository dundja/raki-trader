import { act, cleanup, render, screen, within } from '@testing-library/react'
import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRouter,
} from '@tanstack/react-router'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WindowCard } from '../src/components/WindowCard'
import type { LiveWindow } from '../src/api/generated/LiveWindow'

const NOW = 1_793_000_000_000 // epoch ms the fake clock is pinned to

function nullWindow(over: Partial<LiveWindow> = {}): LiveWindow {
  return {
    asset: 'btc',
    slug: 'btc-updown-5m-1793000100',
    start_ts: 1_793_000_100,
    end_ts: 1_793_000_400,
    secs_left: 0,
    price_to_beat: null,
    twap: null,
    twap_ts_ms: null,
    chainlink: null,
    binance_mid_usd: null,
    coinbase_mid: null,
    up_bid: null,
    up_ask: null,
    down_bid: null,
    down_ask: null,
    ...over,
  }
}

function fullWindow(over: Partial<LiveWindow> = {}): LiveWindow {
  return nullWindow({
    price_to_beat: 80_000,
    twap: 80_040,
    twap_ts_ms: NOW - 2000,
    chainlink: 80_041.5,
    binance_mid_usd: 80_039.25,
    coinbase_mid: 80_038,
    up_bid: '0.56',
    up_ask: '0.57',
    down_bid: '0.43',
    down_ask: '0.44',
    ...over,
  })
}

async function renderCard(ui: ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const root = createRootRoute({
    component: () => (
      <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>
    ),
  })
  const router = createRouter({
    routeTree: root,
    defaultNotFoundComponent: () => null,
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  await act(async () => {
    render(<RouterProvider router={router} />)
  })
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(NOW)
  window.scrollTo = vi.fn()
  // the sparkline seed: no history yet
  vi.stubGlobal(
    'fetch',
    vi.fn(() => Promise.resolve(new Response('[]', { status: 200 }))),
  )
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('WindowCard', () => {
  it('live card renders all-null window', async () => {
    await renderCard(<WindowCard window={nullWindow()} />)
    const card = screen.getByTestId('window-card-btc')
    expect(card.textContent).not.toMatch(/NaN/)
    expect(card.textContent).not.toMatch(/Invalid/)
    expect(card.textContent).not.toMatch(/undefined|null/)
    for (const label of [
      'Price to beat',
      'TWAP',
      'Chainlink',
      'Binance (USD)',
      'Coinbase',
      'Up bid / ask',
      'Down bid / ask',
      'TWAP age',
    ]) {
      const row = within(card).getByText(label).closest('div')!
      expect(row.textContent).toContain('—')
    }
    expect(within(card).getByTestId('twap-delta')).toHaveTextContent('—')
    expect(
      within(card).getByRole('link', { name: 'btc-updown-5m-1793000100' }),
    ).toHaveAttribute('href', '/windows/btc-updown-5m-1793000100')
  })

  it('countdown clamps after end', async () => {
    await renderCard(
      <WindowCard window={nullWindow({ end_ts: NOW / 1000 - 30 })} />,
    )
    const card = screen.getByTestId('window-card-btc')
    expect(card).toHaveTextContent('ended, awaiting resolution')
    expect(screen.getByTestId('countdown').textContent).not.toMatch(/\d/)
  })

  it('countdown ticks every 250 ms', async () => {
    await renderCard(
      <WindowCard window={nullWindow({ end_ts: NOW / 1000 + 65 })} />,
    )
    const countdown = screen.getByTestId('countdown')
    expect(countdown).toHaveTextContent('1:05')
    act(() => {
      vi.advanceTimersByTime(5000)
    })
    expect(countdown).toHaveTextContent('1:00')
    act(() => {
      vi.advanceTimersByTime(60_000)
    })
    expect(countdown).toHaveTextContent('ended, awaiting resolution')
  })

  it('twap delta colours', async () => {
    await renderCard(<WindowCard window={fullWindow({ twap: 80_040 })} />)
    const up = screen.getByTestId('twap-delta')
    expect(up).toHaveAttribute('data-dir', 'up')
    expect(up).toHaveTextContent('+40.00')
    expect(up).toHaveTextContent('+5.0 bp')
    cleanup()
    await renderCard(<WindowCard window={fullWindow({ twap: 79_960 })} />)
    const down = screen.getByTestId('twap-delta')
    expect(down).toHaveAttribute('data-dir', 'down')
    expect(down).toHaveTextContent('-40.00')
    cleanup()
    // a tie resolves Up
    await renderCard(<WindowCard window={fullWindow({ twap: 80_000 })} />)
    expect(screen.getByTestId('twap-delta')).toHaveAttribute('data-dir', 'up')
  })

  it('shows prices, books in cents and the TWAP age', async () => {
    await renderCard(<WindowCard window={fullWindow()} />)
    const card = screen.getByTestId('window-card-btc')
    expect(card).toHaveTextContent('80,000.00')
    expect(card).toHaveTextContent('80,041.50')
    expect(card).toHaveTextContent('56.0¢ / 57.0¢')
    expect(card).toHaveTextContent('43.0¢ / 44.0¢')
    expect(card).toHaveTextContent('2s ago')
  })

  it('seeds the sparkline from the series and appends live points', async () => {
    const series = [
      { slug: 's', ts: 1_792_999_990, twap: 79_990 },
      { slug: 's', ts: 1_792_999_991, twap: null },
      { slug: 's', ts: 1_792_999_992, twap: 80_010 },
    ]
    vi.stubGlobal(
      'fetch',
      vi.fn(() =>
        Promise.resolve(new Response(JSON.stringify(series), { status: 200 })),
      ),
    )
    await renderCard(<WindowCard window={fullWindow({ twap_ts_ms: NOW })} />)
    expect(fetch).toHaveBeenCalledWith(
      'http://127.0.0.1:8787/api/windows/btc-updown-5m-1793000100/series',
    )
    const spark = screen.getByTestId('sparkline')
    // 2 non-null seed points + the live one
    expect(spark).toHaveAttribute('data-points', '3')
  })
})

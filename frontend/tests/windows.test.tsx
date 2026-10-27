import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRouter,
} from '@tanstack/react-router'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { SeriesRow } from '../src/api/generated/SeriesRow'
import type { WindowRecord } from '../src/api/generated/WindowRecord'
import { toChartPoints, toQuotePoints } from '../src/components/WindowChart'
import { MatchBadge, nextBefore, WindowsPage } from '../src/routes/windows.index'
import { WindowDetailPage } from '../src/routes/windows.$slug'

const START = 1_793_000_100

function rec(i: number, over: Partial<WindowRecord> = {}): WindowRecord {
  const start = START - i * 300
  return {
    slug: `btc-updown-5m-${start}`,
    asset: 'btc',
    start_ts: start,
    end_ts: start + 300,
    event_id: null,
    condition_id: null,
    up_token: null,
    down_token: null,
    price_to_beat_ours: 80_000,
    final_twap_ours: 80_040.5,
    outcome_ours: 'Up',
    price_to_beat_gamma: 80_000,
    final_price_gamma: 80_040.5,
    twap_match: true,
    winner: 'Up',
    resolved_at: start + 326,
    resolution_seen_ms: (start + 326) * 1000,
    twap_source: 'polybolt',
    coverage_secs: 300,
    status: 'complete',
    session_id: 's',
    updated_ms: 0,
    ...over,
  }
}

function page(from: number, n: number): WindowRecord[] {
  return Array.from({ length: n }, (_, k) => rec(from + k))
}

function series(over: Partial<SeriesRow> = {}): SeriesRow {
  return {
    slug: 'btc-updown-5m-1793000100',
    ts: START,
    twap: null,
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

function json(body: unknown): Promise<Response> {
  return Promise.resolve(new Response(JSON.stringify(body), { status: 200 }))
}

async function renderPage(ui: ReactNode) {
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

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

beforeEach(() => {
  window.scrollTo = vi.fn()
})

describe('windows list', () => {
  it('windows list paginates with before', async () => {
    const first = page(0, 50)
    const second = page(50, 50)
    const fetchMock = vi.fn((url: string) =>
      json(url.includes('before=') ? second : first),
    )
    vi.stubGlobal('fetch', fetchMock)

    await renderPage(<WindowsPage asset={null} onAssetChange={() => {}} />)
    expect(await screen.findAllByTestId(/^window-row-/)).toHaveLength(50)
    expect(String(fetchMock.mock.calls[0][0])).toMatch(/\/api\/windows\?limit=50$/)

    // asset=all: the cursor is last start_ts + 1 so a split start_ts group is not skipped.
    const last = first[49].start_ts + 1
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Load more' }))
    })
    expect(fetchMock).toHaveBeenCalledTimes(2)
    expect(String(fetchMock.mock.calls[1][0])).toMatch(
      new RegExp(`/api/windows\\?limit=50&before=${last}$`),
    )
    await waitFor(() =>
      expect(screen.getAllByTestId(/^window-row-/)).toHaveLength(100),
    )
  })

  it('all-assets paging does not lose rows of a start_ts group split by the page boundary', async () => {
    const T = START - 1000
    const mk = (asset: 'btc' | 'eth' | 'sol', start: number) =>
      rec(0, { asset, slug: `${asset}-updown-5m-${start}`, start_ts: start })
    // Page 1: 48 btc rows, then only sol and btc of group T (eth of T is cut off by the limit).
    const first = [
      ...Array.from({ length: 48 }, (_, k) => mk('btc', START - k * 300 + 100)),
      mk('sol', T),
      mk('btc', T),
    ]
    // Backend semantics: start_ts < before. The next page re-includes the whole T group.
    const second = [mk('eth', T), mk('sol', T), mk('btc', T), mk('btc', T - 300)]
    const fetchMock = vi.fn((url: string) =>
      json(url.includes('before=') ? second : first),
    )
    vi.stubGlobal('fetch', fetchMock)

    await renderPage(<WindowsPage asset={null} onAssetChange={() => {}} />)
    expect(await screen.findAllByTestId(/^window-row-/)).toHaveLength(50)
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Load more' }))
    })
    expect(String(fetchMock.mock.calls[1][0])).toMatch(
      new RegExp(`before=${T + 1}$`),
    )
    await waitFor(() =>
      expect(screen.getAllByTestId(/^window-row-/)).toHaveLength(52),
    )
    expect(screen.getByTestId(`window-row-eth-updown-5m-${T}`)).toBeTruthy()
  })

  it('nextBefore: exact cursor per asset, +1 for all, guarded against a single-group page', () => {
    const rows = [rec(0), rec(1)]
    const ts = rows[1].start_ts
    expect(nextBefore('btc', rows, null)).toBe(ts)
    expect(nextBefore(null, rows, null)).toBe(ts + 1)
    expect(nextBefore(null, rows, ts + 100)).toBe(ts + 1)
    // The previous cursor was already ts + 1 and the page still ends at ts: step back to make progress.
    expect(nextBefore(null, rows, ts + 1)).toBe(ts)
  })

  it('passes the asset filter to the API and hides Load more on a short page', async () => {
    const fetchMock = vi.fn((_url: string) => json(page(0, 3)))
    vi.stubGlobal('fetch', fetchMock)
    await renderPage(<WindowsPage asset="eth" onAssetChange={() => {}} />)
    expect(await screen.findAllByTestId(/^window-row-/)).toHaveLength(3)
    expect(String(fetchMock.mock.calls[0][0])).toMatch(
      /\/api\/windows\?asset=eth&limit=50$/,
    )
    expect(screen.queryByRole('button', { name: 'Load more' })).toBeNull()
  })

  it('asset filter reports the chosen asset', async () => {
    vi.stubGlobal('fetch', vi.fn(() => json([])))
    const onChange = vi.fn()
    await renderPage(<WindowsPage asset={null} onAssetChange={onChange} />)
    fireEvent.change(screen.getByLabelText('Asset'), { target: { value: 'sol' } })
    expect(onChange).toHaveBeenCalledWith('sol')
    fireEvent.change(screen.getByLabelText('Asset'), { target: { value: 'all' } })
    expect(onChange).toHaveBeenLastCalledWith(null)
  })

  it('renders a null-heavy row without NaN, Invalid Date or null', async () => {
    const early = rec(0, {
      price_to_beat_ours: null,
      final_twap_ours: null,
      outcome_ours: null,
      price_to_beat_gamma: null,
      final_price_gamma: null,
      twap_match: null,
      winner: null,
      resolved_at: null,
      resolution_seen_ms: null,
      twap_source: null,
      coverage_secs: 0,
      status: 'upcoming',
    })
    vi.stubGlobal('fetch', vi.fn(() => json([early])))
    await renderPage(<WindowsPage asset={null} onAssetChange={() => {}} />)
    const row = await screen.findByTestId(`window-row-${early.slug}`)
    expect(row.textContent).not.toMatch(/NaN|Invalid|undefined|null/)
    expect(within(row).getByText('upcoming')).toBeInTheDocument()
  })

  it('shows latency in seconds and coverage as a percent', async () => {
    const r = rec(0, { resolution_seen_ms: (START + 300 + 26) * 1000 + 400, coverage_secs: 150 })
    vi.stubGlobal('fetch', vi.fn(() => json([r])))
    await renderPage(<WindowsPage asset={null} onAssetChange={() => {}} />)
    const row = await screen.findByTestId(`window-row-${r.slug}`)
    expect(row).toHaveTextContent('26.4 s')
    expect(row).toHaveTextContent('50%')
  })
})

describe('MatchBadge', () => {
  it('match badge states', () => {
    render(
      <>
        <MatchBadge match={true} status="complete" />
        <MatchBadge match={false} status="complete" />
        <MatchBadge match={null} status="timed_out" />
        <MatchBadge match={null} status="resolved" />
      </>,
    )
    const ok = screen.getByLabelText('TWAP matches Gamma')
    expect(ok).toHaveTextContent('✓')
    expect(ok).toHaveClass('badge-ok')
    const bad = screen.getByLabelText('TWAP differs from Gamma')
    expect(bad).toHaveTextContent('✗')
    expect(bad).toHaveClass('badge-bad')
    const na = screen.getByLabelText('No comparison available')
    expect(na).toHaveTextContent('—')
    const pending = screen.getByLabelText('Match pending')
    expect(pending).toHaveTextContent('…')
  })
})

describe('window detail', () => {
  it('detail renders empty-series message', async () => {
    const w = rec(0)
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string) => json(url.endsWith('/series') ? [] : w)),
    )
    await renderPage(<WindowDetailPage slug={w.slug} />)
    expect(
      await screen.findByText(
        'No series recorded for this window (app was not running).',
      ),
    ).toBeInTheDocument()
    expect(screen.getByText(w.slug)).toBeInTheDocument()
  })

  it('shows a not-found message for an unknown slug', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.resolve(new Response('not found', { status: 404 }))),
    )
    await renderPage(<WindowDetailPage slug="nope" />)
    expect(await screen.findByText(/Could not load window/)).toBeInTheDocument()
  })
})

describe('WindowChart data', () => {
  it('chart maps ts to seconds into window', () => {
    const pts = toChartPoints(
      [series({ ts: START + 5, twap: 80_001, binance_mid_usd: 80_002 })],
      START,
    )
    expect(pts[0].x).toBe(5)
    expect(pts[0].twap).toBe(80_001)
    expect(pts[0].binance).toBe(80_002)
    expect(pts[0].chainlink).toBeNull()
  })

  it('drops samples outside 0..300 s', () => {
    const pts = toChartPoints(
      [series({ ts: START - 1 }), series({ ts: START }), series({ ts: START + 300 }), series({ ts: START + 301 })],
      START,
    )
    expect(pts.map((p) => p.x)).toEqual([0, 300])
  })

  it('quotes convert decimal strings to cents', () => {
    const pts = toQuotePoints(
      [series({ ts: START + 2, up_bid: '0.56', up_ask: '0.575' }), series({ ts: START + 3 })],
      START,
    )
    expect(pts[0]).toEqual({ x: 2, bid: 56, ask: 57.5 })
    expect(pts[1]).toEqual({ x: 3, bid: null, ask: null })
  })
})

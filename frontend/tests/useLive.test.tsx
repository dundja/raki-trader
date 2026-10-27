import { act, cleanup, render, renderHook, screen } from '@testing-library/react'
import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRouter,
} from '@tanstack/react-router'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { LiveProvider, useLive } from '../src/api/useLive'
import { AppShell } from '../src/routes/__root'
import type { LiveSnapshot } from '../src/api/generated/LiveSnapshot'

function snapshot(
  mode: 'running' | 'paused' | 'killed' = 'running',
  generated_ms = 1_000,
): LiveSnapshot {
  return {
    generated_ms,
    session: { id: 's', started_ms: 0, version: '0.1.0' },
    health: {} as LiveSnapshot['health'],
    control: {
      mode,
      stop_kind: null,
      reason: null,
      needs_ack: false,
      since_ms: 0,
    },
    windows: [],
  }
}

class MockWebSocket {
  static instances: MockWebSocket[] = []
  url: string
  onopen: (() => void) | null = null
  onmessage: ((ev: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  closed = false

  constructor(url: string) {
    this.url = url
    MockWebSocket.instances.push(this)
  }
  close() {
    this.closed = true
  }
  // test helpers
  open() {
    this.onopen?.()
  }
  push(s: LiveSnapshot) {
    this.onmessage?.({ data: JSON.stringify(s) })
  }
  drop() {
    this.onclose?.()
  }
}

const wrapper = ({ children }: { children: ReactNode }) => (
  <LiveProvider>{children}</LiveProvider>
)

beforeEach(() => {
  MockWebSocket.instances = []
  vi.useFakeTimers()
  vi.stubGlobal('WebSocket', MockWebSocket)
  window.scrollTo = vi.fn() // jsdom does not implement it; the router calls it on navigation
  // The one-shot seed from GET /api/live never resolves unless a test says so.
  vi.stubGlobal('fetch', vi.fn(() => new Promise(() => {})))
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('useLive', () => {
  it('useLive parses pushed snapshot', () => {
    const { result } = renderHook(() => useLive(), { wrapper })
    expect(result.current.status).toBe('connecting')
    expect(result.current.snapshot).toBeNull()
    const ws = MockWebSocket.instances[0]
    expect(ws.url).toBe('ws://127.0.0.1:8787/api/ws')

    act(() => ws.open())
    act(() => ws.push(snapshot('paused')))

    expect(result.current.status).toBe('open')
    expect(result.current.snapshot?.control.mode).toBe('paused')
    expect(result.current.lastMessageMs).not.toBeNull()
  })

  it('useLive reconnects after close', () => {
    const { result } = renderHook(() => useLive(), { wrapper })
    const first = MockWebSocket.instances[0]
    act(() => first.open())
    act(() => first.push(snapshot()))

    act(() => first.drop())
    expect(result.current.status).toBe('reconnecting')
    // the last snapshot stays available
    expect(result.current.snapshot).not.toBeNull()
    expect(MockWebSocket.instances).toHaveLength(1)

    act(() => {
      vi.advanceTimersByTime(999)
    })
    expect(MockWebSocket.instances).toHaveLength(1)
    act(() => {
      vi.advanceTimersByTime(1)
    })
    expect(MockWebSocket.instances).toHaveLength(2)

    // a failed retry backs off: 2 s next, and the status stays "reconnecting"
    act(() => MockWebSocket.instances[1].drop())
    expect(result.current.status).toBe('reconnecting')
    act(() => {
      vi.advanceTimersByTime(1999)
    })
    expect(MockWebSocket.instances).toHaveLength(2)
    act(() => {
      vi.advanceTimersByTime(1)
    })
    expect(MockWebSocket.instances).toHaveLength(3)

    // success resets the backoff to 1 s
    act(() => MockWebSocket.instances[2].open())
    expect(result.current.status).toBe('open')
    act(() => MockWebSocket.instances[2].drop())
    act(() => {
      vi.advanceTimersByTime(1000)
    })
    expect(MockWebSocket.instances).toHaveLength(4)
  })

  it('caps the backoff at 10 s', () => {
    renderHook(() => useLive(), { wrapper })
    // fail repeatedly without ever opening: 1,2,4,8,10,10 s
    for (const delay of [1000, 2000, 4000, 8000, 10000, 10000]) {
      const n = MockWebSocket.instances.length
      act(() => MockWebSocket.instances[n - 1].drop())
      act(() => {
        vi.advanceTimersByTime(delay - 1)
      })
      expect(MockWebSocket.instances).toHaveLength(n)
      act(() => {
        vi.advanceTimersByTime(1)
      })
      expect(MockWebSocket.instances).toHaveLength(n + 1)
    }
  })

  it('does not reconnect after unmount', () => {
    const { unmount } = renderHook(() => useLive(), { wrapper })
    const ws = MockWebSocket.instances[0]
    act(() => ws.drop())
    unmount()
    act(() => {
      vi.advanceTimersByTime(20_000)
    })
    expect(MockWebSocket.instances).toHaveLength(1)
  })

  it('seeds from GET /api/live and lets a newer pushed frame win', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify(snapshot('running', 500)), { status: 200 }),
      ),
    )
    const { result } = renderHook(() => useLive(), { wrapper })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0)
    })
    expect(result.current.snapshot?.generated_ms).toBe(500)
    // seeding alone does not mean the socket is open
    expect(result.current.status).toBe('connecting')

    act(() => MockWebSocket.instances[0].push(snapshot('paused', 900)))
    expect(result.current.snapshot?.generated_ms).toBe(900)
  })

  it('useLive throws outside a LiveProvider', () => {
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {})
    expect(() => renderHook(() => useLive())).toThrow(/LiveProvider/)
    spy.mockRestore()
  })
})

async function renderShell() {
  const root = createRootRoute({
    component: () => (
      <LiveProvider>
        <AppShell />
      </LiveProvider>
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

describe('app shell', () => {
  it('shell shows mode pill', async () => {
    await renderShell()
    const ws = MockWebSocket.instances[0]
    act(() => ws.open())
    act(() => ws.push(snapshot('paused')))
    expect(screen.getByText('PAUSED')).toBeInTheDocument()
    expect(screen.getByText('live')).toBeInTheDocument()
    act(() => ws.push(snapshot('killed', 2000)))
    expect(screen.getByText('KILLED')).toBeInTheDocument()
    act(() => ws.push(snapshot('running', 3000)))
    expect(screen.getByText('RUNNING')).toBeInTheDocument()
  })

  it('shell has Live, Health and Windows nav links', async () => {
    await renderShell()
    expect(screen.getByRole('link', { name: 'Live' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Health' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Windows' })).toBeInTheDocument()
  })

  it('shows the unreachable banner after 3 s of reconnecting with no snapshot', async () => {
    await renderShell()
    act(() => MockWebSocket.instances[0].drop())
    expect(screen.getByText('reconnecting…')).toBeInTheDocument()
    expect(screen.queryByText(/Backend unreachable/)).toBeNull()
    act(() => {
      vi.advanceTimersByTime(3001)
    })
    expect(
      screen.getByText('Backend unreachable at http://127.0.0.1:8787'),
    ).toBeInTheDocument()
  })

  it('keeps the last snapshot, shows reconnecting, and no banner once a snapshot exists', async () => {
    await renderShell()
    const ws = MockWebSocket.instances[0]
    act(() => ws.open())
    act(() => ws.push(snapshot('paused')))
    act(() => ws.drop())
    act(() => {
      vi.advanceTimersByTime(5000)
    })
    expect(screen.getByText('reconnecting…')).toBeInTheDocument()
    expect(screen.getByText('PAUSED')).toBeInTheDocument()
    expect(screen.queryByText(/Backend unreachable/)).toBeNull()
  })
})

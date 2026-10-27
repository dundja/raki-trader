import {
  HeadContent,
  Link,
  Outlet,
  Scripts,
  createRootRoute,
} from '@tanstack/react-router'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { useEffect, useState } from 'react'

import { API_URL } from '../api/client'
import type { TradingMode } from '../api/generated/TradingMode'
import { LiveProvider, useLive, type LiveStatus } from '../api/useLive'
import appCss from '../styles/app.css?url'

export const Route = createRootRoute({
  head: () => ({
    meta: [
      {
        charSet: 'utf-8',
      },
      {
        name: 'viewport',
        content: 'width=device-width, initial-scale=1',
      },
      {
        title: 'Raki Trader',
      },
    ],
    links: [
      {
        rel: 'stylesheet',
        href: appCss,
      },
    ],
  }),
  component: RootLayout,
  shellComponent: RootDocument,
})

function RootDocument({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <HeadContent />
      </head>
      <body>
        {children}

        <Scripts />
      </body>
    </html>
  )
}

function RootLayout() {
  // One query client and one WebSocket per browser tab, created once.
  const [queryClient] = useState(() => new QueryClient())
  return (
    <QueryClientProvider client={queryClient}>
      <LiveProvider>
        <AppShell />
      </LiveProvider>
    </QueryClientProvider>
  )
}

/** After this long in "reconnecting" with nothing to show, say the backend is unreachable. */
const UNREACHABLE_AFTER_MS = 3000

const NAV: ReadonlyArray<{ to: string; label: string }> = [
  { to: '/', label: 'Live' },
  { to: '/health', label: 'Health' },
  { to: '/windows', label: 'Windows' },
]

const STATUS_LABEL: Record<LiveStatus, string> = {
  open: 'live',
  reconnecting: 'reconnecting…',
  connecting: 'connecting',
}

const STATUS_CLASS: Record<LiveStatus, string> = {
  open: 'badge-ok',
  reconnecting: 'badge-warn',
  connecting: 'badge-idle',
}

const MODE_CLASS: Record<TradingMode, string> = {
  running: 'badge-ok',
  paused: 'badge-warn',
  killed: 'badge-bad',
}

/** Nav, connection badge, trading-mode pill and backend-unreachable banner around the page. */
export function AppShell() {
  const { snapshot, status } = useLive()
  const [unreachable, setUnreachable] = useState(false)

  const waiting = status === 'reconnecting' && snapshot === null
  useEffect(() => {
    if (!waiting) {
      setUnreachable(false)
      return
    }
    const t = setTimeout(() => setUnreachable(true), UNREACHABLE_AFTER_MS)
    return () => clearTimeout(t)
  }, [waiting])

  const mode = snapshot?.control.mode
  // Keep the last snapshot on screen when the socket drops, but grey it out.
  const stale = snapshot !== null && status !== 'open'

  return (
    <div className="shell">
      {unreachable && (
        <div className="banner banner-bad" role="alert">
          Backend unreachable at {API_URL}
        </div>
      )}
      <header className="topbar">
        <span className="brand">Raki Trader</span>
        <nav aria-label="Main">
          {NAV.map(({ to, label }) => (
            <Link
              key={to}
              to={to as '/'}
              activeOptions={{ exact: to === '/' }}
              activeProps={{ className: 'active' }}
            >
              {label}
            </Link>
          ))}
        </nav>
        <div className="topbar-status">
          <span className={`badge ${STATUS_CLASS[status]}`}>
            {STATUS_LABEL[status]}
          </span>
          {mode && (
            <span className={`badge ${MODE_CLASS[mode]}`}>
              {mode.toUpperCase()}
            </span>
          )}
        </div>
      </header>
      <main className={stale ? 'content stale' : 'content'}>
        <Outlet />
      </main>
    </div>
  )
}

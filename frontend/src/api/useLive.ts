import {
  createContext,
  createElement,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { API_URL, getJson } from './client'
import type { LiveSnapshot } from './generated/LiveSnapshot'

export type LiveStatus = 'connecting' | 'open' | 'reconnecting'

export type LiveValue = {
  snapshot: LiveSnapshot | null
  status: LiveStatus
  /** Local receive time (epoch ms) of the last pushed frame. */
  lastMessageMs: number | null
  /**
   * The socket is open but no frame arrived for longer than {@link STALE_AFTER_MS}. The backend
   * pushes every second, so this means its loop stalled: the snapshot on screen is old.
   */
  stale: boolean
}

const BACKOFF_BASE_MS = 1000
const BACKOFF_MAX_MS = 10_000
/** An open socket with no frame for this long is stale (the backend pushes every 1 s). */
export const STALE_AFTER_MS = 5000
const STALE_CHECK_MS = 1000

/** Delay before reconnect attempt number `attempt` (0-based): 1 s, 2 s, 4 s, 8 s, then 10 s. */
export function backoffMs(attempt: number): number {
  return Math.min(BACKOFF_BASE_MS * 2 ** attempt, BACKOFF_MAX_MS)
}

const LiveContext = createContext<LiveValue | null>(null)

/** Keeps the newer of two snapshots, so a slow GET /api/live never overwrites a pushed frame. */
function newer(prev: LiveSnapshot | null, next: LiveSnapshot): LiveSnapshot {
  return prev && prev.generated_ms > next.generated_ms ? prev : next
}

/** Owns the single WebSocket for the app. Mount it once, near the root. */
export function LiveProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<LiveSnapshot | null>(null)
  const [status, setStatus] = useState<LiveStatus>('connecting')
  const [lastMessageMs, setLastMessageMs] = useState<number | null>(null)
  const [stale, setStale] = useState(false)
  /** Latest frame, or the socket's open time before its first frame. */
  const lastSignOfLife = useRef<number | null>(null)

  // The watchdog: an open socket alone does not mean the data is current.
  useEffect(() => {
    if (status !== 'open') {
      setStale(false)
      return
    }
    const check = () => {
      const last = lastSignOfLife.current
      setStale(last !== null && Date.now() - last > STALE_AFTER_MS)
    }
    check()
    const id = setInterval(check, STALE_CHECK_MS)
    return () => clearInterval(id)
  }, [status])

  useEffect(() => {
    let disposed = false
    let attempt = 0
    let socket: WebSocket | null = null
    let timer: ReturnType<typeof setTimeout> | null = null

    const connect = () => {
      const ws = new WebSocket(`${API_URL.replace(/^http/, 'ws')}/api/ws`)
      socket = ws
      let lost = false

      // onerror and onclose can both fire for one failure; schedule one retry per socket.
      const lose = () => {
        if (disposed || lost) return
        lost = true
        setStatus('reconnecting')
        timer = setTimeout(connect, backoffMs(attempt))
        attempt += 1
      }

      ws.onopen = () => {
        if (disposed) return
        attempt = 0
        lastSignOfLife.current = Date.now()
        setStatus('open')
      }
      ws.onmessage = (ev: MessageEvent) => {
        if (disposed) return
        let next: LiveSnapshot
        try {
          next = JSON.parse(String(ev.data)) as LiveSnapshot
        } catch {
          return // ignore a malformed frame; the next one replaces it
        }
        const at = Date.now()
        lastSignOfLife.current = at
        setSnapshot((prev) => newer(prev, next))
        setLastMessageMs(at)
        setStale(false)
      }
      ws.onerror = lose
      ws.onclose = lose
    }

    connect()

    // Seed once so the page has data before the first push (the backend pushes on a timer).
    getJson<LiveSnapshot>('/api/live')
      .then((seed) => {
        if (!disposed) setSnapshot((prev) => newer(prev, seed))
      })
      .catch(() => {
        // backend down: the socket's reconnect loop and the shell banner report it
      })

    return () => {
      disposed = true
      if (timer) clearTimeout(timer)
      if (socket) {
        socket.onopen = socket.onmessage = socket.onerror = socket.onclose = null
        socket.close()
      }
    }
  }, [])

  const value = useMemo(
    () => ({ snapshot, status, lastMessageMs, stale }),
    [snapshot, status, lastMessageMs, stale],
  )
  return createElement(LiveContext.Provider, { value }, children)
}

export function useLive(): LiveValue {
  const ctx = useContext(LiveContext)
  if (!ctx) throw new Error('useLive must be used inside <LiveProvider>')
  return ctx
}

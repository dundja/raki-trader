import { createFileRoute } from '@tanstack/react-router'

import { useLive } from '../api/useLive'
import type { Asset } from '../api/generated/Asset'
import { WindowCard } from '../components/WindowCard'

export const Route = createFileRoute('/')({ component: LivePage })

const ASSET_ORDER: readonly Asset[] = ['btc', 'eth', 'sol']

function LivePage() {
  const { snapshot } = useLive()
  if (snapshot === null) {
    return <p className="muted">Waiting for data from the backend…</p>
  }
  const windows = [...snapshot.windows].sort(
    (a, b) => ASSET_ORDER.indexOf(a.asset) - ASSET_ORDER.indexOf(b.asset),
  )
  if (windows.length === 0) {
    return <p className="muted">No open windows right now.</p>
  }
  return (
    <div className="cards">
      {windows.map((w) => (
        <WindowCard key={w.slug} window={w} />
      ))}
    </div>
  )
}

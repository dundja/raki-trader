import { createFileRoute } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'

import { getJson, qk } from '../api/client'
import type { RecordingStats } from '../api/generated/RecordingStats'
import { useLive } from '../api/useLive'
import { ControlPanel } from '../components/ControlPanel'
import { FeedTable } from '../components/FeedTable'
import { fmtAgo, fmtInt, fmtMb, fmtUtc } from '../components/fmt'
import { GateProgress } from '../components/GateProgress'
import { SafetyPanel } from '../components/SafetyPanel'
import { useNow } from '../components/useNow'

export const Route = createFileRoute('/health')({ component: HealthPage })

const STATS_REFETCH_MS = 10_000

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="card">
      <h2>{title}</h2>
      {children}
    </section>
  )
}

function HealthPage() {
  const { snapshot } = useLive()
  const now = useNow(1000)
  const stats = useQuery({
    queryKey: qk.stats,
    queryFn: () => getJson<RecordingStats>('/api/stats'),
    refetchInterval: STATS_REFETCH_MS,
  })

  if (snapshot === null) {
    return <p className="muted">Waiting for data from the backend…</p>
  }
  const { health, control } = snapshot
  const { clock, geoblock, recorder } = health

  return (
    <div className="health">
      <Section title="Trading control">
        <ControlPanel control={control} />
      </Section>
      <Section title="Safety checks">
        <SafetyPanel safety={health.safety} />
      </Section>
      <Section title="Recording progress">
        <GateProgress stats={stats.data ?? null} />
      </Section>
      <Section title="Feeds">
        <FeedTable feeds={health.feeds} now={now} />
      </Section>
      <Section title="Clock">
        <p>
          Offset{' '}
          <strong className={clock.ok ? 'ok' : 'bad'}>
            {clock.offset_ms === null ? '—' : `${clock.offset_ms.toFixed(1)} ms`}
          </strong>{' '}
          (threshold {clock.max_offset_ms} ms)
        </p>
        <p className="muted" title={clock.checked_ms === null ? undefined : fmtUtc(clock.checked_ms)}>
          checked {fmtAgo(clock.checked_ms, now)}
        </p>
      </Section>
      <Section title="Geoblock">
        <p>
          Country {geoblock.country ?? '—'} · blocked{' '}
          {geoblock.blocked === null ? '—' : geoblock.blocked ? 'yes' : 'no'}
        </p>
        <p className="muted" title={geoblock.checked_ms === null ? undefined : fmtUtc(geoblock.checked_ms)}>
          checked {fmtAgo(geoblock.checked_ms, now)}
        </p>
      </Section>
      <Section title="Recorder">
        <dl className="fields">
          <div className="field">
            <dt>File</dt>
            <dd className="mono">{recorder.file ?? '—'}</dd>
          </div>
          <div className="field">
            <dt>Records</dt>
            <dd>{fmtInt(recorder.records)}</dd>
          </div>
          <div className="field">
            <dt>Size</dt>
            <dd>{fmtMb(recorder.bytes)}</dd>
          </div>
          <div className="field">
            <dt>Dropped</dt>
            <dd className={recorder.dropped > 0 ? 'bad' : undefined}>{fmtInt(recorder.dropped)}</dd>
          </div>
          <div className="field">
            <dt>Last flush</dt>
            <dd>{fmtAgo(recorder.last_flush_ms, now)}</dd>
          </div>
        </dl>
      </Section>
      <Section title="Counts">
        <dl className="fields">
          <div className="field">
            <dt>Sleep events</dt>
            <dd>{fmtInt(health.sleep_events)}</dd>
          </div>
          <div className="field">
            <dt>Book resyncs</dt>
            <dd>{fmtInt(health.book_resyncs)}</dd>
          </div>
        </dl>
      </Section>
    </div>
  )
}

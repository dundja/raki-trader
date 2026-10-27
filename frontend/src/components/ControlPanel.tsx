import { useEffect, useState } from 'react'

import { postControl } from '../api/client'
import type { ControlState } from '../api/generated/ControlState'
import { fmtUtc } from './fmt'

type Action = 'pause' | 'resume' | 'kill'

const TITLE: Record<Action, string> = {
  pause: 'Pause trading',
  resume: 'Resume trading',
  kill: 'Kill',
}

const TOAST_MS = 4000

/**
 * The dashboard's only mutating controls. Nothing is applied optimistically: after a response the
 * panel only reports it, and the next WebSocket snapshot (via `control`) drives the displayed state.
 */
export function ControlPanel({ control }: { control: ControlState }) {
  const [open, setOpen] = useState<Action | null>(null)
  const [toast, setToast] = useState<string | null>(null)

  useEffect(() => {
    if (toast === null) return
    const id = setTimeout(() => setToast(null), TOAST_MS)
    return () => clearTimeout(id)
  }, [toast])

  return (
    <div className="control-panel">
      <p>
        Mode:{' '}
        <strong data-testid="control-mode" className={`badge badge-${control.mode === 'running' ? 'ok' : control.mode === 'paused' ? 'warn' : 'bad'}`}>
          {control.mode.toUpperCase()}
        </strong>
      </p>
      {control.mode !== 'running' && (
        <p>
          Stopped: {control.stop_kind ?? '—'}
          {control.reason ? ` — ${control.reason}` : ''}{' '}
          <span className="muted" title={fmtUtc(control.since_ms)}>
            (since {new Date(control.since_ms).toLocaleTimeString()})
          </span>
        </p>
      )}
      <div className="buttons">
        <button type="button" disabled={control.mode !== 'running'} onClick={() => setOpen('pause')}>
          Pause
        </button>
        <button type="button" disabled={control.mode === 'running'} onClick={() => setOpen('resume')}>
          Resume
        </button>
        <button type="button" className="danger" onClick={() => setOpen('kill')}>
          Kill
        </button>
      </div>
      {open && (
        <ConfirmDialog
          key={open}
          action={open}
          control={control}
          onClose={() => setOpen(null)}
          onDone={(mode) => {
            setOpen(null)
            setToast(`Mode: ${mode}`)
          }}
        />
      )}
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </div>
  )
}

function ConfirmDialog({
  action,
  control,
  onClose,
  onDone,
}: {
  action: Action
  control: ControlState
  onClose: () => void
  onDone: (mode: string) => void
}) {
  const [reason, setReason] = useState('')
  const [ack, setAck] = useState(false)
  const [typed, setTyped] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !busy) onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [busy, onClose])

  const ackNeeded = action === 'resume' && control.needs_ack
  const ready =
    !busy && (action !== 'kill' || typed === 'KILL') && (!ackNeeded || ack)

  const submit = async () => {
    if (!ready) return
    setBusy(true)
    setError(null)
    try {
      const res =
        action === 'pause'
          ? await postControl('pause', { reason: reason.trim() || null, acknowledge: null })
          : action === 'resume'
            ? await postControl('resume', { reason: null, acknowledge: ack })
            : await postControl('kill', { reason: null, acknowledge: null })
      if (!res.ok) {
        // A refusal is a normal answer: show it and keep the dialog open.
        setError(res.error ?? 'The backend refused the request.')
        setBusy(false)
        return
      }
      onDone(res.state.mode)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  return (
    <div className="dialog-backdrop">
      <div className="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title">
        <h3 id="dialog-title">{TITLE[action]}</h3>

        {action === 'pause' && (
          <p>
            <label>
              Reason (optional)
              <input type="text" value={reason} onChange={(e) => setReason(e.target.value)} />
            </label>
          </p>
        )}

        {action === 'resume' && (
          <>
            <p>
              Why it stopped: <strong>{control.stop_kind ?? '—'}</strong>
              {control.reason ? ` — ${control.reason}` : ''}
            </p>
            {ackNeeded && (
              <label className="check">
                <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} />
                I acknowledge: {control.reason ?? control.stop_kind ?? 'this stop'}
              </label>
            )}
          </>
        )}

        {action === 'kill' && (
          <p>
            <label>
              Type KILL to confirm
              <input type="text" value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" />
            </label>
            <span className="muted"> Pauses trading and cancels every resting order.</span>
          </p>
        )}

        {error && (
          <p className="dialog-error" role="alert">
            {error}
          </p>
        )}

        <div className="buttons">
          <button type="button" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            type="button"
            className={action === 'kill' ? 'danger' : 'primary'}
            disabled={!ready}
            onClick={submit}
          >
            Confirm
          </button>
        </div>
      </div>
    </div>
  )
}

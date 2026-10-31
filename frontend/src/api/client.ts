import type { ControlRequest } from './generated/ControlRequest'
import type { ControlResponse } from './generated/ControlResponse'

/** Base URL of the Rust API. Override with VITE_API_URL. */
export const API_URL: string =
  import.meta.env.VITE_API_URL ?? 'http://127.0.0.1:8787'

export class ApiError extends Error {
  status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }
}

async function failure(res: Response, what: string): Promise<ApiError> {
  let detail = ''
  try {
    detail = (await res.text()).trim().slice(0, 200)
  } catch {
    // body unreadable; the status alone is enough
  }
  return new ApiError(
    res.status,
    `${what} failed: HTTP ${res.status}${detail ? ` ${detail}` : ''}`,
  )
}

/** GET a JSON route. Throws ApiError on any non-2xx status. */
export async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(`${API_URL}${path}`)
  if (!res.ok) throw await failure(res, `GET ${path}`)
  return (await res.json()) as T
}

/**
 * The only mutating call the dashboard makes. A backend refusal (for example resume while safety
 * checks fail) is HTTP 200 with `ok: false` and an `error`; callers show that, so it is returned,
 * not thrown. Transport and server errors throw ApiError.
 */
export async function postControl(
  action: 'pause' | 'resume' | 'kill',
  body: ControlRequest,
): Promise<ControlResponse> {
  const path = `/api/control/${action}`
  const res = await fetch(`${API_URL}${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!res.ok) throw await failure(res, `POST ${path}`)
  return (await res.json()) as ControlResponse
}

export const qk = {
  live: ['live'],
  health: ['health'],
  stats: ['stats'],
  windows: (asset: string | null, before: number | null) =>
    ['windows', asset, before] as const,
  window: (slug: string) => ['window', slug] as const,
  series: (slug: string) => ['series', slug] as const,
  predictions: (slug: string) => ['predictions', slug] as const,
  windowShadow: (slug: string) => ['window-shadow', slug] as const,
  shadowSummary: ['shadow-summary'],
  shadowFills: ['shadow-fills'],
  calibration: (checkpoint: number, asset: string | null) =>
    ['calibration', checkpoint, asset] as const,
} as const

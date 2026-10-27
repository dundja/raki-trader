import { afterEach, describe, expect, it, vi } from 'vitest'
import { API_URL, ApiError, getJson, postControl, qk } from '../src/api/client'
import {
  fmtAgo,
  fmtCountdown,
  fmtPrice,
  fmtProb,
  fmtUtc,
} from '../src/components/fmt'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('fmt', () => {
  it('fmtCountdown formats m:ss and clamps at zero', () => {
    expect(fmtCountdown(83.2)).toBe('1:23')
    expect(fmtCountdown(9)).toBe('0:09')
    expect(fmtCountdown(-4)).toBe('ended')
    expect(fmtCountdown(0)).toBe('ended')
    expect(fmtCountdown(Number.NaN)).toBe('—')
  })

  it('fmtProb shows cents with one decimal', () => {
    expect(fmtProb('0.95')).toBe('95.0¢')
    expect(fmtProb('0.925')).toBe('92.5¢')
    expect(fmtProb(null)).toBe('—')
    expect(fmtProb(undefined)).toBe('—')
    expect(fmtProb('abc')).toBe('—')
  })

  it('fmtPrice renders null as a dash and never NaN', () => {
    expect(fmtPrice(null)).toBe('—')
    expect(fmtPrice(undefined)).toBe('—')
    expect(fmtPrice('not a number')).toBe('—')
    expect(fmtPrice('67123.456')).toBe('67,123.46')
    expect(fmtPrice(0.5, 4)).toBe('0.5000')
  })

  it('fmtAgo is relative to now', () => {
    expect(fmtAgo(null, 10_000)).toBe('—')
    expect(fmtAgo(7_000, 10_000)).toBe('3s ago')
    expect(fmtAgo(10_500, 10_000)).toBe('0s ago')
    expect(fmtAgo(10_000 - 125_000, 10_000)).toBe('2m ago')
  })

  it('fmtUtc is an ISO-like UTC string', () => {
    expect(fmtUtc(0)).toBe('1970-01-01 00:00:00 UTC')
    expect(fmtUtc(Number.NaN)).toBe('—')
  })
})

describe('api client', () => {
  it('defaults the API base to the local backend', () => {
    expect(API_URL).toBe('http://127.0.0.1:8787')
  })

  it('getJson throws ApiError with the status on a 404', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response('nope', { status: 404 })),
    )
    const err = await getJson('/api/windows/missing').catch((e: unknown) => e)
    expect(err).toBeInstanceOf(ApiError)
    expect((err as ApiError).status).toBe(404)
  })

  it('getJson returns parsed JSON and calls the API base', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response(JSON.stringify({ a: 1 }), { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    await expect(getJson<{ a: number }>('/api/live')).resolves.toEqual({ a: 1 })
    expect(fetchMock.mock.calls[0][0]).toBe(`${API_URL}/api/live`)
  })

  it('postControl sends JSON with a JSON content-type', async () => {
    const body = { ok: true, state: {}, error: null }
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response(JSON.stringify(body), { status: 200 }))
    vi.stubGlobal('fetch', fetchMock)
    const res = await postControl('resume', {
      reason: 'ok',
      acknowledge: true,
    })
    expect(res).toEqual(body)
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe(`${API_URL}/api/control/resume`)
    expect(init.method).toBe('POST')
    expect(new Headers(init.headers).get('content-type')).toBe(
      'application/json',
    )
    expect(JSON.parse(init.body as string)).toEqual({
      reason: 'ok',
      acknowledge: true,
    })
  })

  it('postControl returns the ControlResponse of a backend refusal (HTTP 200, ok false)', async () => {
    const body = { ok: false, state: {}, error: 'feeds stale' }
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response(JSON.stringify(body), { status: 200 })),
    )
    await expect(
      postControl('resume', { reason: null, acknowledge: null }),
    ).resolves.toEqual(body)
  })

  it('postControl throws ApiError on a non-2xx response', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response('bad gateway', { status: 502 })),
    )
    const err = await postControl('kill', {
      reason: null,
      acknowledge: null,
    }).catch((e: unknown) => e)
    expect(err).toBeInstanceOf(ApiError)
    expect((err as ApiError).status).toBe(502)
  })

  it('query keys are stable', () => {
    expect(qk.live).toEqual(['live'])
    expect(qk.windows('btc', null)).toEqual(['windows', 'btc', null])
    expect(qk.window('s')).toEqual(['window', 's'])
    expect(qk.series('s')).toEqual(['series', 's'])
  })
})

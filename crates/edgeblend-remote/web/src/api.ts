// Typed client for the edgeblend remote API (crates/edgeblend-remote/src/server.rs).
// Every call carries the session token from the page URL (`?t=`).

export interface PatternInfo {
  /** serde name, sent back in `{ op: 'pattern', value: name }` */
  name: string
  label: string
}

export type Corners = [[number, number], [number, number], [number, number], [number, number]]

export interface State {
  rev: number
  projectors: number
  overlap_frac: number
  /** One entry per projector, wall order left to right. */
  muted: boolean[]
  pattern: string
  patterns: PatternInfo[]
  mirror: boolean
  reverse_order: boolean
  blend_ramps: boolean
  gamma: number
  curve: number
  canvas: [number, number]
  wall_m: [number, number]
  geometry: Corners[]
}

/** Corner `c0..c3` (TL, TR, BR, BL), edge `e0..e3` (top, right, bottom, left) or `all`. */
export type Handle = 'c0' | 'c1' | 'c2' | 'c3' | 'e0' | 'e1' | 'e2' | 'e3' | 'all'

export type Op =
  | { op: 'pattern'; value: string }
  | { op: 'set_mute'; projector: number; muted: boolean }
  | { op: 'nudge'; projector: number; handle: Handle; dx: number; dy: number }
  | { op: 'set_corners'; projector: number; corners: Corners }
  | { op: 'blend_ramps'; value: boolean }
  | { op: 'gamma'; value: number }
  | { op: 'curve'; value: number }
  | { op: 'undo' }

export type Source = 'desktop' | 'remote' | 'restore' | 'import' | 'auto'

export interface HistoryRow {
  id: string
  created: string
  created_ms: number
  milestone: boolean
  name: string | null
  source: Source
  broken: boolean
}

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message)
  }
}

export const token: string = new URLSearchParams(window.location.search).get('t') ?? ''

const TIMEOUT_MS = 5000

async function call<T>(method: 'GET' | 'POST', path: string, body?: unknown): Promise<T> {
  const ctrl = new AbortController()
  const timer = setTimeout(() => ctrl.abort(), TIMEOUT_MS)
  try {
    const res = await fetch(`${path}?t=${encodeURIComponent(token)}`, {
      method,
      signal: ctrl.signal,
      cache: 'no-store',
      headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
    if (!res.ok) {
      const text = (await res.text().catch(() => '')).slice(0, 200)
      throw new ApiError(res.status, text || res.statusText)
    }
    return (await res.json()) as T
  } catch (e) {
    if (e instanceof ApiError) throw e
    throw new ApiError(0, 'cannot reach edgeblend')
  } finally {
    clearTimeout(timer)
  }
}

export const api = {
  state: () => call<State>('GET', '/api/state'),
  edit: (op: Op) => call<State>('POST', '/api/edit', op),
  history: () => call<HistoryRow[]>('GET', '/api/history'),
  milestone: (name: string) => call<{ id: string }>('POST', '/api/milestone', name.trim() ? { name } : {}),
  restore: (id: string) => call<State>('POST', '/api/restore', { id }),
}

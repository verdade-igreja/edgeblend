// Geometry helpers shared by the Adjust tab (mirror crates/edgeblend-remote/src/apply.rs).
import type { Corners, Handle } from '@/api'

/** Max |dx|/|dy| per request (server clamps to the same, `NUDGE_MAX`). */
export const NUDGE_MAX = 0.1
const CORNER_MIN = -2
const CORNER_MAX = 3

/** Corner indices a handle moves (TL=0, TR=1, BR=2, BL=3). */
export const HANDLE_CORNERS: Record<Handle, number[]> = {
  c0: [0],
  c1: [1],
  c2: [2],
  c3: [3],
  e0: [0, 1],
  e1: [1, 2],
  e2: [2, 3],
  e3: [3, 0],
  all: [0, 1, 2, 3],
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v))

/** Same move the server makes, for an instant (optimistic) redraw. */
export function nudged(c: Corners, h: Handle, dx: number, dy: number): Corners {
  const out = c.map((p) => [p[0], p[1]]) as Corners
  for (const i of HANDLE_CORNERS[h]) {
    out[i] = [clamp(out[i][0] + dx, CORNER_MIN, CORNER_MAX), clamp(out[i][1] + dy, CORNER_MIN, CORNER_MAX)]
  }
  return out
}

/** Signal pixels each projector gets (1920x1080 split into 3), like the desktop editor. */
const SIGNAL_TILE: [number, number] = [640, 1080]

/** One projector signal pixel in wall units (the desktop arrow-key step). */
export function projectorPixel(c: Corners | undefined): [number, number] {
  const fallback: [number, number] = [0.001, 0.001]
  if (!c) return fallback
  const len = (a: number[], b: number[]) => Math.hypot(a[0] - b[0], a[1] - b[1])
  const w = 0.5 * (len(c[0], c[1]) + len(c[3], c[2]))
  const h = 0.5 * (len(c[0], c[3]) + len(c[1], c[2]))
  if (!(w > 0) || !(h > 0)) return fallback
  return [w / SIGNAL_TILE[0], h / SIGNAL_TILE[1]]
}

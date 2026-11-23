<script setup lang="ts">
// The calibrated wall quads as they land on the wall (normalized 0..1 wall coordinates).
// The dashed box is the wall. Vertical scale is stretched on very wide walls so the
// corners stay visible on a phone. Tap a quad to select that projector.
import { computed } from 'vue'
import type { Corners, Handle } from '@/api'
import { HANDLE_CORNERS } from '@/lib/geometry'

const props = defineProps<{
  geometry: Corners[]
  wall: [number, number]
  muted: boolean[]
  /** Selected projector, or 'all'. */
  target: number | 'all'
  handle: Handle
}>()
const emit = defineEmits<{ select: [index: number] }>()

const W = 1000
const HUES = [205, 25, 140, 280, 55, 340]

const view = computed(() => {
  const aspect = props.wall[0] > 0 ? props.wall[1] / props.wall[0] : 9 / 16
  const H = W * Math.max(aspect, 0.4)
  const pts = props.geometry.map((q) => q.map(([x, y]) => [x * W, y * H] as [number, number]))
  let [x0, y0, x1, y1] = [0, 0, W, H]
  for (const q of pts) for (const [x, y] of q) {
    x0 = Math.min(x0, x)
    y0 = Math.min(y0, y)
    x1 = Math.max(x1, x)
    y1 = Math.max(y1, y)
  }
  const pad = 24
  return { H, pts, box: `${x0 - pad} ${y0 - pad} ${x1 - x0 + 2 * pad} ${y1 - y0 + 2 * pad}` }
})

const isTarget = (i: number) => props.target === 'all' || props.target === i
const poly = (q: [number, number][]) => q.map((p) => p.join(',')).join(' ')

/** Highlight of the selected handle: dots for corners, bars for edges/whole. */
function marks(q: [number, number][]) {
  const idx = HANDLE_CORNERS[props.handle]
  const dots = idx.map((i) => q[i])
  const lines: [number, number][][] = []
  if (props.handle.startsWith('e')) lines.push([q[idx[0]], q[idx[1]]])
  return { dots, lines }
}
</script>

<template>
  <svg :viewBox="view.box" class="w-full select-none touch-manipulation" role="group" aria-label="Projector shapes on the wall">
    <rect x="0" y="0" :width="W" :height="view.H" fill="none" stroke="#737373" stroke-width="3" stroke-dasharray="16 12" />
    <g
      v-for="(q, i) in view.pts"
      :key="i"
      class="cursor-pointer"
      role="button"
      :aria-label="`Projector ${i + 1}`"
      :aria-pressed="isTarget(i)"
      @click="emit('select', i)"
    >
      <polygon
        :points="poly(q)"
        :fill="muted[i] ? '#0a0a0a' : `hsl(${HUES[i % HUES.length]} 75% 55%)`"
        :fill-opacity="isTarget(i) ? 0.45 : 0.2"
        :stroke="isTarget(i) ? '#fafafa' : `hsl(${HUES[i % HUES.length]} 80% 65%)`"
        :stroke-width="isTarget(i) ? 5 : 2.5"
        stroke-linejoin="round"
      />
      <text
        :x="(q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4"
        :y="(q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4"
        text-anchor="middle"
        dominant-baseline="central"
        fill="#fafafa"
        :fill-opacity="isTarget(i) ? 1 : 0.6"
        font-size="56"
        font-weight="700"
        pointer-events="none"
      >
        P{{ i + 1 }}
      </text>
    </g>
    <g v-for="(q, i) in view.pts" :key="`m${i}`" pointer-events="none">
      <template v-if="isTarget(i)">
        <line
          v-for="(l, k) in marks(q).lines"
          :key="k"
          :x1="l[0][0]"
          :y1="l[0][1]"
          :x2="l[1][0]"
          :y2="l[1][1]"
          stroke="#facc15"
          stroke-width="12"
          stroke-linecap="round"
        />
        <circle v-for="(p, k) in marks(q).dots" :key="k" :cx="p[0]" :cy="p[1]" r="14" fill="#facc15" stroke="#0a0a0a" stroke-width="3" />
      </template>
    </g>
  </svg>
</template>

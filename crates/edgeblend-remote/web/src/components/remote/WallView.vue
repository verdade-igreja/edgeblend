<script setup lang="ts">
// Schematic wall: N equal rectangles side by side, each overlapping the next by
// `overlap` (the idea of the layout, not the calibrated geometry). Projector index is the
// wall position left to right; mirror / reverse order only remap the video signal.
//
// Each projector is its own <g> (id `proj-<i>`, data-* state) so the next slice can add
// edge handles inside it and an arrow pad bound to the selection.
import { computed } from 'vue'

const props = defineProps<{
  projectors: number
  overlap: number
  muted: boolean[]
  selected: number | null
}>()
const emit = defineEmits<{ select: [index: number | null] }>()

const W = 1000
const ASPECT = 9 / 16 // per projector; schematic only

const layout = computed(() => {
  const n = Math.max(1, Math.min(6, props.projectors))
  const o = Math.min(0.8, Math.max(0, props.overlap || 0))
  const w = W / (1 + (n - 1) * (1 - o))
  const h = w * ASPECT
  const rects = Array.from({ length: n }, (_, i) => ({ i, x: i * w * (1 - o), w, h }))
  return { rects, h, pad: 8 }
})

// Distinct hues so overlaps read as mixed colour.
const HUES = [205, 25, 140, 280, 55, 340]

function fill(i: number) {
  return props.muted[i] ? '#0a0a0a' : `hsl(${HUES[i % HUES.length]} 75% 55%)`
}

function toggleSelect(i: number) {
  emit('select', props.selected === i ? null : i)
}
</script>

<template>
  <svg
    :viewBox="`${-layout.pad} ${-layout.pad} ${W + 2 * layout.pad} ${layout.h + 2 * layout.pad}`"
    class="w-full select-none touch-manipulation"
    role="group"
    aria-label="Projectors on the wall"
  >
    <g
      v-for="r in layout.rects"
      :id="`proj-${r.i}`"
      :key="r.i"
      class="cursor-pointer outline-none"
      role="button"
      tabindex="0"
      :aria-label="`Projector ${r.i + 1}${muted[r.i] ? ' (off)' : ''}`"
      :aria-pressed="selected === r.i"
      :data-projector="r.i"
      :data-muted="muted[r.i] ? 'true' : 'false'"
      :data-selected="selected === r.i ? 'true' : 'false'"
      @click="toggleSelect(r.i)"
      @keydown.enter.prevent="toggleSelect(r.i)"
      @keydown.space.prevent="toggleSelect(r.i)"
    >
      <rect
        :x="r.x"
        :y="0"
        :width="r.w"
        :height="r.h"
        rx="6"
        :fill="fill(r.i)"
        :fill-opacity="muted[r.i] ? 0.9 : 0.45"
        :stroke="selected === r.i ? '#fafafa' : muted[r.i] ? '#404040' : `hsl(${HUES[r.i % HUES.length]} 80% 65%)`"
        :stroke-width="selected === r.i ? 6 : 3"
        :stroke-dasharray="muted[r.i] ? '14 10' : undefined"
      />
      <text
        :x="r.x + r.w / 2"
        :y="r.h / 2"
        text-anchor="middle"
        dominant-baseline="central"
        :fill="muted[r.i] ? '#737373' : '#fafafa'"
        font-size="64"
        font-weight="700"
        pointer-events="none"
      >
        P{{ r.i + 1 }}
      </text>
      <text
        v-if="muted[r.i]"
        :x="r.x + r.w / 2"
        :y="r.h / 2 + 62"
        text-anchor="middle"
        dominant-baseline="central"
        fill="#737373"
        font-size="36"
        pointer-events="none"
      >
        OFF
      </text>
    </g>
  </svg>
</template>

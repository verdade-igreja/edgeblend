<script setup lang="ts">
// Geometry nudging from the phone: pick a projector (or all), a part (corner, edge, whole),
// then the arrow pad. Hold an arrow to repeat; repeats are merged into one request while
// the previous one is in flight, so a slow network never queues up a backlog.
import { computed, onUnmounted, ref, watch } from 'vue'
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Undo2 } from '@lucide/vue'
import type { Handle, Op, State } from '@/api'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { NUDGE_MAX, nudged, projectorPixel } from '@/lib/geometry'

const props = defineProps<{
  state: State
  target: number | 'all'
  handle: Handle
  disabled?: boolean
  /** Sends one op; resolves true when applied. */
  send: (op: Op, optimistic?: (s: State) => State) => Promise<boolean>
}>()
const emit = defineEmits<{ 'update:target': [t: number | 'all']; 'update:handle': [h: Handle] }>()

// 3x3 grid laid out like the rectangle itself.
const PARTS: { h: Handle; label: string }[] = [
  { h: 'c0', label: 'Top-left' },
  { h: 'e0', label: 'Top edge' },
  { h: 'c1', label: 'Top-right' },
  { h: 'e3', label: 'Left edge' },
  { h: 'all', label: 'Whole' },
  { h: 'e1', label: 'Right edge' },
  { h: 'c3', label: 'Bottom-left' },
  { h: 'e2', label: 'Bottom edge' },
  { h: 'c2', label: 'Bottom-right' },
]

const STEPS = [
  { m: 0.25, label: '¼ px' },
  { m: 1, label: '1 px' },
  { m: 5, label: '5 px' },
  { m: 20, label: '20 px' },
]

function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key)
    return v === null ? fallback : (JSON.parse(v) as T)
  } catch {
    return fallback
  }
}
function save(key: string, v: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(v))
  } catch {
    /* private mode */
  }
}

const step = ref<number>(load('eb.step', 1))
/** Like the desktop "Arrows move the picture" (default on): arrows push the image. */
const movePicture = ref<boolean>(load('eb.movePicture', true))
watch(step, (v) => save('eb.step', v))
watch(movePicture, (v) => save('eb.movePicture', v))

const n = computed(() => props.state.geometry.length)

/** Step in wall units: one signal pixel of the target projector (average for all). */
const pixel = computed<[number, number]>(() => {
  const g = props.state.geometry
  const list = props.target === 'all' ? g : [g[props.target]]
  const px = list.map(projectorPixel)
  const avg = (k: 0 | 1) => px.reduce((a, p) => a + p[k], 0) / Math.max(1, px.length)
  return [avg(0), avg(1)]
})

// ---- nudge queue -------------------------------------------------------------------------
let pending = { dx: 0, dy: 0 }
let inflight = false

function add(dirX: number, dirY: number) {
  if (props.disabled) return
  const sign = movePicture.value ? -1 : 1
  pending.dx += sign * dirX * pixel.value[0] * step.value
  pending.dy += sign * dirY * pixel.value[1] * step.value
  void flush()
}

async function flush() {
  if (inflight || (pending.dx === 0 && pending.dy === 0)) return
  const take = (v: number) => Math.max(-NUDGE_MAX, Math.min(NUDGE_MAX, v))
  const dx = take(pending.dx)
  const dy = take(pending.dy)
  pending = { dx: pending.dx - dx, dy: pending.dy - dy }
  const handle = props.handle
  const target = props.target
  const op: Op =
    target === 'all'
      ? { op: 'nudge_all', handle, dx, dy }
      : { op: 'nudge', projector: target, handle, dx, dy }
  inflight = true
  let ok = false
  try {
    ok = await props.send(op, (s) => ({
      ...s,
      geometry: s.geometry.map((q, i) => (target === 'all' || target === i ? nudged(q, handle, dx, dy) : q)),
    }))
  } finally {
    inflight = false
  }
  if (ok) void flush()
  else pending = { dx: 0, dy: 0 }
}

// ---- hold to repeat ----------------------------------------------------------------------
let holdTimer: ReturnType<typeof setTimeout> | undefined
let repeatTimer: ReturnType<typeof setInterval> | undefined

function press(dirX: number, dirY: number) {
  release()
  add(dirX, dirY)
  holdTimer = setTimeout(() => {
    repeatTimer = setInterval(() => add(dirX, dirY), 80)
  }, 400)
}
function release() {
  clearTimeout(holdTimer)
  clearInterval(repeatTimer)
  holdTimer = repeatTimer = undefined
}
onUnmounted(release)

const ARROWS = [
  { x: 0, y: -1, icon: ArrowUp, label: 'Up', cls: 'col-start-2 row-start-1' },
  { x: -1, y: 0, icon: ArrowLeft, label: 'Left', cls: 'col-start-1 row-start-2' },
  { x: 1, y: 0, icon: ArrowRight, label: 'Right', cls: 'col-start-3 row-start-2' },
  { x: 0, y: 1, icon: ArrowDown, label: 'Down', cls: 'col-start-2 row-start-3' },
]

function undo() {
  release()
  pending = { dx: 0, dy: 0 }
  void props.send({ op: 'undo' })
}

const targetName = computed(() => (props.target === 'all' ? 'All projectors' : `P${props.target + 1}`))
const partName = computed(() => PARTS.find((p) => p.h === props.handle)?.label ?? '')
</script>

<template>
  <div class="space-y-4">
    <div class="grid gap-2" :style="{ gridTemplateColumns: `repeat(${n + 1}, minmax(0, 1fr))` }">
      <Button
        v-for="i in n"
        :key="i"
        :variant="target === i - 1 ? 'default' : 'outline'"
        :aria-pressed="target === i - 1"
        class="h-12 text-base"
        @click="emit('update:target', i - 1)"
      >
        P{{ i }}
      </Button>
      <Button
        :variant="target === 'all' ? 'default' : 'outline'"
        :aria-pressed="target === 'all'"
        class="h-12 text-base"
        @click="emit('update:target', 'all')"
      >
        All
      </Button>
    </div>

    <div class="grid grid-cols-3 gap-1.5 rounded-xl border border-dashed border-border p-1.5">
      <Button
        v-for="p in PARTS"
        :key="p.h"
        :variant="handle === p.h ? 'default' : 'outline'"
        :aria-pressed="handle === p.h"
        class="h-12 whitespace-normal px-1 text-sm leading-tight"
        @click="emit('update:handle', p.h)"
      >
        {{ p.label }}
      </Button>
    </div>

    <p class="text-center text-sm text-muted-foreground">
      Moving <span class="font-semibold text-foreground">{{ partName }}</span> of
      <span class="font-semibold text-foreground">{{ targetName }}</span>
    </p>

    <div class="mx-auto grid w-60 grid-cols-3 grid-rows-3 gap-2">
      <Button
        v-for="a in ARROWS"
        :key="a.label"
        variant="secondary"
        :class="['size-[4.5rem] touch-none', a.cls]"
        :aria-label="a.label"
        :disabled="disabled"
        @pointerdown.prevent="press(a.x, a.y)"
        @pointerup="release"
        @pointercancel="release"
        @pointerleave="release"
        @contextmenu.prevent
        @keydown.enter.prevent="add(a.x, a.y)"
        @keydown.space.prevent="add(a.x, a.y)"
      >
        <component :is="a.icon" class="size-8" />
      </Button>
      <Button
        variant="outline"
        class="col-start-2 row-start-2 size-[4.5rem] flex-col gap-0.5 text-xs"
        aria-label="Undo"
        :disabled="disabled"
        @click="undo"
      >
        <Undo2 class="size-6" />
        Undo
      </Button>
    </div>

    <div class="grid grid-cols-4 gap-2">
      <Button
        v-for="s in STEPS"
        :key="s.m"
        :variant="step === s.m ? 'default' : 'outline'"
        :aria-pressed="step === s.m"
        class="h-11"
        @click="step = s.m"
      >
        {{ s.label }}
      </Button>
    </div>

    <label class="flex min-h-12 items-center justify-between gap-3 rounded-xl border px-4">
      <span class="text-sm">
        <span class="font-semibold">Arrows move the picture</span>
        <span class="block text-muted-foreground">{{ movePicture ? 'Pushes the image on the wall' : 'Moves the projector outline' }}</span>
      </span>
      <Switch v-model="movePicture" />
    </label>
  </div>
</template>

<script setup lang="ts">
// Blend ramps on/off, gamma and curve. Sliders show the value while dragging and send it
// once on release.
import { ref, watch } from 'vue'
import type { Op, State } from '@/api'
import { Switch } from '@/components/ui/switch'

const props = defineProps<{
  state: State
  disabled?: boolean
  send: (op: Op, optimistic?: (s: State) => State) => Promise<boolean>
}>()

const gamma = ref(props.state.gamma)
const curve = ref(props.state.curve)
let dragging = false
watch(
  () => [props.state.gamma, props.state.curve],
  ([g, c]) => {
    if (dragging) return
    gamma.value = g
    curve.value = c
  },
)

function commit(op: 'gamma' | 'curve', v: number) {
  dragging = false
  void props.send({ op, value: v }, (s) => ({ ...s, [op]: v }))
}
function ramps(on: boolean) {
  void props.send({ op: 'blend_ramps', value: on }, (s) => ({ ...s, blend_ramps: on }))
}
</script>

<template>
  <div class="space-y-4">
    <label class="flex min-h-12 items-center justify-between rounded-xl border px-4">
      <span class="font-semibold">Blend ramps</span>
      <Switch :model-value="state.blend_ramps" :disabled="disabled" @update:model-value="ramps" />
    </label>
    <label class="block space-y-1">
      <span class="flex justify-between text-sm"><span class="font-semibold">Gamma</span>{{ gamma.toFixed(2) }}</span>
      <input
        v-model.number="gamma"
        type="range"
        min="1"
        max="3"
        step="0.05"
        class="h-8 w-full accent-foreground"
        :disabled="disabled"
        @input="dragging = true"
        @change="commit('gamma', gamma)"
      />
    </label>
    <label class="block space-y-1">
      <span class="flex justify-between text-sm"><span class="font-semibold">Curve</span>{{ curve.toFixed(2) }}</span>
      <input
        v-model.number="curve"
        type="range"
        min="0.5"
        max="4"
        step="0.05"
        class="h-8 w-full accent-foreground"
        :disabled="disabled"
        @input="dragging = true"
        @change="commit('curve', curve)"
      />
    </label>
  </div>
</template>

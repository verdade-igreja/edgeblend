<script setup lang="ts">
import type { PatternInfo } from '@/api'
import { Button } from '@/components/ui/button'

defineProps<{ patterns: PatternInfo[]; current: string; disabled?: boolean }>()
const emit = defineEmits<{ pick: [name: string] }>()
</script>

<template>
  <div class="grid grid-cols-2 gap-2">
    <Button
      v-for="p in patterns"
      :key="p.name"
      :variant="p.name === current ? 'default' : 'outline'"
      :aria-pressed="p.name === current"
      :disabled="disabled"
      class="h-auto min-h-14 whitespace-normal px-3 py-2 text-base leading-tight"
      :class="p.name === 'desktop' ? 'col-span-2' : ''"
      @click="emit('pick', p.name)"
    >
      {{ p.label }}
    </Button>
  </div>
</template>

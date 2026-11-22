<script setup lang="ts">
import { Switch } from '@/components/ui/switch'

defineProps<{ muted: boolean[]; selected: number | null; disabled?: boolean }>()
const emit = defineEmits<{ toggle: [index: number, muted: boolean]; select: [index: number] }>()
</script>

<template>
  <div class="grid grid-cols-1 gap-2 sm:grid-cols-3">
    <label
      v-for="(m, i) in muted"
      :key="i"
      :data-projector="i"
      class="flex min-h-14 cursor-pointer items-center justify-between rounded-xl border px-4 transition-colors"
      :class="[
        m ? 'border-border bg-neutral-950 text-muted-foreground' : 'border-border bg-card',
        selected === i ? 'ring-2 ring-foreground/70' : '',
      ]"
    >
      <span class="text-base font-semibold">
        P{{ i + 1 }}
        <span class="ml-1 text-sm font-normal" :class="m ? 'text-amber-500' : 'text-muted-foreground'">
          {{ m ? 'off' : 'on' }}
        </span>
      </span>
      <Switch
        class="scale-150 origin-right"
        :model-value="!m"
        :disabled="disabled"
        :aria-label="`Projector ${i + 1} on`"
        @update:model-value="(on: boolean) => emit('toggle', i, !on)"
      />
    </label>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { toast } from 'vue-sonner'
import { StarIcon } from '@lucide/vue'
import { api, ApiError, type HistoryRow, type Source, type State } from '@/api'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { Badge } from '@/components/ui/badge'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

const emit = defineEmits<{ restored: [state: State] }>()

const rows = ref<HistoryRow[]>([])
const loading = ref(false)
const unavailable = ref(false)
const name = ref('')
const saving = ref(false)
const milestonesOnly = ref(false)
const confirm = ref<HistoryRow | null>(null)
const restoring = ref(false)

const shown = computed(() => (milestonesOnly.value ? rows.value.filter((r) => r.milestone) : rows.value))

const SOURCE: Record<Source, string> = {
  desktop: 'desktop',
  remote: 'phone',
  restore: 'restore',
  import: 'import',
  auto: 'auto',
}

function when(r: HistoryRow) {
  return new Date(r.created_ms).toLocaleString(undefined, {
    day: 'numeric',
    month: 'short',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

function message(e: unknown) {
  return e instanceof ApiError ? e.message : 'something went wrong'
}

async function load() {
  loading.value = true
  try {
    rows.value = await api.history()
    unavailable.value = false
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) unavailable.value = true
    else toast.error(`History: ${message(e)}`)
  } finally {
    loading.value = false
  }
}

async function saveMilestone() {
  saving.value = true
  try {
    await api.milestone(name.value)
    toast.success('Milestone saved')
    name.value = ''
    await load()
  } catch (e) {
    toast.error(`Save failed: ${message(e)}`)
  } finally {
    saving.value = false
  }
}

async function restore() {
  const row = confirm.value
  if (!row) return
  restoring.value = true
  try {
    emit('restored', await api.restore(row.id))
    toast.success('Version restored')
    confirm.value = null
    await load()
  } catch (e) {
    toast.error(`Restore failed: ${message(e)}`)
  } finally {
    restoring.value = false
  }
}

onMounted(load)
defineExpose({ load })
</script>

<template>
  <div class="space-y-4">
    <p v-if="unavailable" class="text-muted-foreground">History is not available.</p>
    <template v-else>
      <form class="flex gap-2" @submit.prevent="saveMilestone">
        <Input
          v-model="name"
          maxlength="80"
          placeholder="Milestone name (optional)"
          class="h-12 flex-1 text-base"
          aria-label="Milestone name"
        />
        <Button type="submit" class="h-12 px-4 text-base" :disabled="saving">
          <StarIcon />Save
        </Button>
      </form>

      <div class="flex items-center justify-between">
        <label class="flex items-center gap-3 text-sm">
          <Switch v-model="milestonesOnly" aria-label="Milestones only" />
          Milestones only
        </label>
        <Button variant="ghost" size="sm" :disabled="loading" @click="load">Refresh</Button>
      </div>

      <ul class="divide-y divide-border rounded-xl border">
        <li v-if="!shown.length" class="p-4 text-sm text-muted-foreground">
          {{ loading ? 'Loading…' : 'No versions yet.' }}
        </li>
        <li
          v-for="r in shown"
          :key="r.id"
          class="flex items-center gap-3 p-3"
          :class="r.milestone ? 'bg-amber-500/5' : ''"
          :data-version="r.id"
        >
          <StarIcon
            class="size-5 shrink-0"
            :class="r.milestone ? 'fill-amber-400 text-amber-400' : 'text-transparent'"
            aria-hidden="true"
          />
          <div class="min-w-0 flex-1">
            <div class="truncate" :class="r.milestone ? 'font-semibold' : ''">
              {{ r.name || (r.milestone ? 'Milestone' : 'Auto save') }}
            </div>
            <div class="flex items-center gap-2 text-xs text-muted-foreground">
              <span>{{ when(r) }}</span>
              <Badge variant="outline" class="text-[10px]">{{ SOURCE[r.source] ?? r.source }}</Badge>
              <Badge v-if="r.broken" variant="destructive" class="text-[10px]">unreadable</Badge>
            </div>
          </div>
          <Button
            variant="outline"
            class="h-11 px-3"
            :disabled="r.broken"
            @click="confirm = r"
          >
            Restore
          </Button>
        </li>
      </ul>
    </template>

    <Dialog :open="confirm !== null" @update:open="(o: boolean) => { if (!o) confirm = null }">
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Restore this version?</DialogTitle>
          <DialogDescription>
            The projection switches to
            <strong>{{ confirm?.name || (confirm?.milestone ? 'Milestone' : 'Auto save') }}</strong>
            from {{ confirm ? when(confirm) : '' }}. The current setup stays in the history.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter class="gap-2">
          <Button variant="outline" class="h-12" @click="confirm = null">Cancel</Button>
          <Button class="h-12" :disabled="restoring" @click="restore">Restore</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </div>
</template>

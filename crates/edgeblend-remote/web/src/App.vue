<script setup lang="ts">
import { computed, ref } from 'vue'
import { toast } from 'vue-sonner'
import 'vue-sonner/style.css'
import { ApiError, token, type State } from '@/api'
import { useRemote } from '@/composables/useRemote'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Toaster } from '@/components/ui/sonner'
import WallView from '@/components/remote/WallView.vue'
import ProjectorSwitches from '@/components/remote/ProjectorSwitches.vue'
import PatternPicker from '@/components/remote/PatternPicker.vue'
import HistoryPanel from '@/components/remote/HistoryPanel.vue'
import GeometryView from '@/components/remote/GeometryView.vue'
import AdjustPanel from '@/components/remote/AdjustPanel.vue'
import BlendPanel from '@/components/remote/BlendPanel.vue'
import type { Handle, Op } from '@/api'

const { state, connection, edit, adopt } = useRemote()

/** Selected projector: tap a rectangle. */
const selected = ref<number | null>(null)
const tab = ref('control')

/** Adjust tab: projector (or all) and the part the arrows move. */
const target = ref<number | 'all'>(0)
const handle = ref<Handle>('all')

function selectFromWall(i: number | null) {
  selected.value = i
  if (i !== null) target.value = i
}

/** Send for the Adjust/Blend panels: true when applied, errors shown as a toast. */
async function send(op: Op, optimistic?: (s: State) => State): Promise<boolean> {
  try {
    const ok = await edit(op, optimistic)
    if (!ok) toast.error('Not applied: cannot reach edgeblend')
    return ok
  } catch (e) {
    toast.error(e instanceof ApiError ? e.message : 'Not applied')
    return false
  }
}

const status = computed(() => {
  switch (connection.value) {
    case 'online':
      return { text: 'connected', cls: 'bg-emerald-500' }
    case 'offline':
      return { text: 'reconnecting…', cls: 'bg-amber-500 animate-pulse' }
    case 'denied':
      return { text: 'link expired', cls: 'bg-red-500' }
    default:
      return { text: 'connecting…', cls: 'bg-neutral-500 animate-pulse' }
  }
})

async function run(op: Parameters<typeof edit>[0], optimistic?: (s: State) => State) {
  try {
    if (!(await edit(op, optimistic))) toast.error('Not applied: cannot reach edgeblend')
  } catch (e) {
    toast.error(e instanceof ApiError ? e.message : 'Not applied')
  }
}

function setMute(i: number, muted: boolean) {
  void run({ op: 'set_mute', projector: i, muted }, (s) => ({
    ...s,
    muted: s.muted.map((m, j) => (j === i ? muted : m)),
  }))
}

function setPattern(name: string) {
  void run({ op: 'pattern', value: name }, (s) => ({ ...s, pattern: name }))
}
</script>

<template>
  <Toaster position="top-center" theme="dark" rich-colors />
  <main class="mx-auto flex min-h-dvh max-w-xl flex-col gap-4 px-4 pt-[max(1rem,env(safe-area-inset-top))] pb-[max(1.5rem,env(safe-area-inset-bottom))]">
    <header class="flex items-center justify-between">
      <h1 class="text-xl font-bold tracking-tight">edgeblend</h1>
      <div class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
        <span class="size-2.5 rounded-full" :class="status.cls" />
        {{ status.text }}
      </div>
    </header>

    <Card v-if="!token || connection === 'denied'" class="border-red-500/40">
      <CardContent class="space-y-2 pt-6">
        <p class="font-semibold">{{ token ? 'This link no longer works.' : 'Missing access token.' }}</p>
        <p class="text-sm text-muted-foreground">
          Open the remote again from the edgeblend tray icon (Remote control…) and scan the new QR code.
          The link changes every time the remote is started.
        </p>
      </CardContent>
    </Card>

    <template v-else-if="state">
      <GeometryView
        v-if="tab === 'adjust'"
        :geometry="state.geometry"
        :wall="state.wall_m"
        :muted="state.muted"
        :target="target"
        :handle="handle"
        @select="target = $event"
      />
      <WallView
        v-else
        :projectors="state.projectors"
        :overlap="state.overlap_frac"
        :muted="state.muted"
        :selected="selected"
        @select="selectFromWall"
      />

      <Tabs v-model="tab" class="gap-4">
        <TabsList class="grid h-12 w-full grid-cols-3">
          <TabsTrigger value="control" class="text-base">Control</TabsTrigger>
          <TabsTrigger value="adjust" class="text-base">Adjust</TabsTrigger>
          <TabsTrigger value="history" class="text-base">History</TabsTrigger>
        </TabsList>

        <TabsContent value="adjust" class="space-y-4">
          <Card>
            <CardHeader><CardTitle>Position</CardTitle></CardHeader>
            <CardContent>
              <AdjustPanel
                v-model:target="target"
                v-model:handle="handle"
                :state="state"
                :disabled="connection !== 'online'"
                :send="send"
              />
            </CardContent>
          </Card>
          <Card>
            <CardHeader><CardTitle>Blend</CardTitle></CardHeader>
            <CardContent>
              <BlendPanel :state="state" :disabled="connection !== 'online'" :send="send" />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="control" class="space-y-4">
          <Card>
            <CardHeader><CardTitle>Projectors</CardTitle></CardHeader>
            <CardContent>
              <ProjectorSwitches
                :muted="state.muted"
                :selected="selected"
                :disabled="connection !== 'online'"
                @toggle="setMute"
              />
            </CardContent>
          </Card>
          <Card>
            <CardHeader><CardTitle>Show</CardTitle></CardHeader>
            <CardContent>
              <PatternPicker
                :patterns="state.patterns"
                :current="state.pattern"
                :disabled="connection !== 'online'"
                @pick="setPattern"
              />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="history">
          <HistoryPanel v-if="tab === 'history'" @restored="adopt" />
        </TabsContent>
      </Tabs>
    </template>

    <p v-else class="py-12 text-center text-muted-foreground">
      {{ connection === 'offline' ? 'Cannot reach edgeblend. Same Wi-Fi as the PC?' : 'Connecting…' }}
    </p>
  </main>
</template>

import { onMounted, onUnmounted, ref, shallowRef } from 'vue'
import { api, ApiError, type Op, type State } from '@/api'

const POLL_MS = 1000

export type Connection = 'connecting' | 'online' | 'offline' | 'denied'

/**
 * Live remote state: polls `/api/state` about once a second and applies edits.
 *
 * Ordering: the server bumps `rev` on every edit (phone or desktop). A response older than
 * the newest one seen is dropped, and so is any poll that was sent before an edit started,
 * so a slow poll can never undo what the user just tapped.
 */
export function useRemote() {
  const state = shallowRef<State | null>(null)
  const connection = ref<Connection>('connecting')
  const busy = ref(0)
  let lastRev = -1
  let epoch = 0 // bumped when an edit starts; polls from an older epoch are stale
  let timer: ReturnType<typeof setTimeout> | undefined
  let polling = false

  function accept(s: State) {
    if (s.rev < lastRev) return
    lastRev = s.rev
    state.value = s
  }

  function fail(e: unknown) {
    connection.value = e instanceof ApiError && e.status === 403 ? 'denied' : 'offline'
  }

  async function poll() {
    if (polling || document.hidden || connection.value === 'denied') return
    polling = true
    const started = epoch
    try {
      const s = await api.state()
      connection.value = 'online'
      if (started === epoch && busy.value === 0) accept(s)
    } catch (e) {
      fail(e)
    } finally {
      polling = false
    }
  }

  function loop() {
    timer = setTimeout(async () => {
      await poll()
      loop()
    }, POLL_MS)
  }

  /** Apply `op`; `optimistic` updates the view at once (rolled back by the next poll on error). */
  async function edit(op: Op, optimistic?: (s: State) => State): Promise<boolean> {
    epoch++
    busy.value++
    if (optimistic && state.value) state.value = optimistic(state.value)
    try {
      accept(await api.edit(op))
      connection.value = 'online'
      return true
    } catch (e) {
      if (e instanceof ApiError && e.status !== 0 && e.status !== 403) {
        connection.value = 'online'
        throw e
      }
      fail(e)
      return false
    } finally {
      busy.value--
      if (busy.value === 0) {
        epoch++
        void poll() // resync (also undoes a failed optimistic change)
      }
    }
  }

  /** Adopt a state returned by another call (e.g. restore). */
  function adopt(s: State) {
    epoch++
    accept(s)
  }

  function onVisible() {
    if (!document.hidden) void poll()
  }

  onMounted(() => {
    void poll()
    loop()
    document.addEventListener('visibilitychange', onVisible)
  })
  onUnmounted(() => {
    clearTimeout(timer)
    document.removeEventListener('visibilitychange', onVisible)
  })

  return { state, connection, busy, edit, adopt, refresh: poll }
}

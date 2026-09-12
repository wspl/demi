import { computed, onScopeDispose, ref, watch } from 'vue'

/** Measures a live interval or a persisted interval; each stopped scope releases its clock. */
export function useElapsedTime(
  startedAt: () => number,
  running: () => boolean,
  endedAt: () => number | null = () => null,
) {
  const now = ref(Date.now())
  let timer: ReturnType<typeof setInterval> | undefined

  function stop(): void {
    if (timer !== undefined) {
      clearInterval(timer)
      timer = undefined
    }
  }

  watch(
    [startedAt, running, endedAt],
    () => {
      stop()
      now.value = Date.now()
      if (running() && endedAt() === null) {
        timer = setInterval(() => {
          now.value = Date.now()
        }, 1000)
      }
    },
    { immediate: true },
  )
  onScopeDispose(stop)

  return computed(() => {
    const start = startedAt()
    const end = endedAt() ?? (running() ? now.value : null)
    if (!Number.isFinite(start) || end === null || !Number.isFinite(end)) {
      return null
    }
    return Math.max(0, end - start)
  })
}

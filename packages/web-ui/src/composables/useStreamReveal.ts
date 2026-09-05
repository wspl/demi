import { onBeforeUnmount, ref, toValue, watch, type MaybeRefOrGetter } from 'vue'
import { alignShown, nextShownText, prefersReducedMotion, STREAM_REVEAL } from '../ui/stream-reveal'

export function useStreamReveal(
  content: MaybeRefOrGetter<string>,
  streaming: MaybeRefOrGetter<boolean>,
) {
  const shown = ref(toValue(content))
  const frontier = ref('')
  let frame = 0
  let lastTs = 0
  let deadline = 0

  function stop(): void {
    if (frame) cancelAnimationFrame(frame)
    frame = 0
    lastTs = 0
    deadline = 0
  }

  function finish(target: string): void {
    shown.value = target
    frontier.value = ''
    stop()
  }

  function tick(ts: number): void {
    const target = toValue(content)
    const live = toValue(streaming)
    shown.value = alignShown(shown.value, target)

    if (prefersReducedMotion()) {
      finish(target)
      return
    }

    if (shown.value === target) {
      frontier.value = ''
      stop()
      return
    }

    const dt = lastTs === 0 ? 16 : Math.min(50, Math.max(8, ts - lastTs))
    lastTs = ts
    if (deadline === 0 || ts > deadline) {
      deadline = ts + (live ? STREAM_REVEAL.maxLagMs : STREAM_REVEAL.flushLagMs)
    }
    const next = nextShownText(shown.value, target, dt, Math.max(dt, deadline - ts))
    frontier.value = next.slice(shown.value.length)
    shown.value = next
    frame = requestAnimationFrame(tick)
  }

  function ensure(): void {
    const target = toValue(content)
    shown.value = alignShown(shown.value, target)
    deadline = 0
    if (prefersReducedMotion() || (!toValue(streaming) && shown.value === target)) {
      finish(target)
      return
    }
    if (shown.value === target) {
      frontier.value = ''
      stop()
      return
    }
    if (!frame) frame = requestAnimationFrame(tick)
  }

  watch([() => toValue(content), () => toValue(streaming)], ensure, { immediate: true })
  onBeforeUnmount(stop)

  return { shown, frontier }
}

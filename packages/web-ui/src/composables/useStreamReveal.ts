import { onBeforeUnmount, ref, shallowRef, toValue, watch, type MaybeRefOrGetter } from 'vue'
import { alignShown, nextStepEnd, STREAM_PACE, stepInterval } from '../ui/stream-reveal'

/** A part of the text the reader was shown at once, `start` to `end` in the source, at `at` (performance.now()). */
export interface RevealStep {
  start: number
  end: number
  at: number
}

/**
 * The part of a streaming text the reader sees, a step at a time
 * (`STREAM_PACE`), and the steps still fading in. `safe` gives the part of a
 * text that renders as it will stay, without markdown left half open; the
 * steps never pass it. When the stream ends, whatever remains shows at once,
 * as its own step. A text that streams in while the tab is hidden shows whole
 * when the tab comes back, since no one watched it arrive.
 */
export function useStreamReveal(
  content: MaybeRefOrGetter<string>,
  streaming: MaybeRefOrGetter<boolean>,
  safe: (text: string) => string,
) {
  // A text already there when the view mounts, as a remount mid-stream, shows at once.
  const shown = ref(toValue(content))
  const steps = shallowRef<RevealStep[]>([])
  let timer: ReturnType<typeof setTimeout> | undefined

  /** Shows `text` up to `end`, the part not shown before as a step when `fade`. */
  function show(end: number, text: string, fade = true): void {
    const start = alignShown(shown.value, text).length
    const now = performance.now()
    const fading = steps.value.filter((step) => now - step.at < STREAM_PACE.fadeMs && step.end <= start)
    steps.value = fade && end > start ? [...fading, { start, end, at: now }] : fading
    shown.value = text.slice(0, end)
  }

  // Whether the text was streaming when it last changed: the end of a stream
  // fades what remains, a text that changes outside one (read whole, edited) shows as it is.
  let live = toValue(streaming)

  function step(): void {
    timer = undefined
    const text = toValue(content)
    if (!toValue(streaming)) {
      show(text.length, text, live)
      live = false
      return
    }
    live = true
    const from = alignShown(shown.value, text).length
    const ceiling = safe(text).length
    if (document.hidden) {
      show(Math.max(from, ceiling), text)
      return
    }
    if (from >= ceiling) {
      show(from, text)
      return
    }
    const end = nextStepEnd(text, from, ceiling, safe)
    show(end, text)
    if (end < ceiling)
      timer = setTimeout(step, stepInterval(ceiling - end))
  }

  // New text steps at once when nothing is waiting, otherwise the running wait
  // takes it; the stream's end shows the rest at once.
  function arrive(): void {
    if (!toValue(streaming)) {
      clearTimeout(timer)
      step()
      return
    }
    if (timer === undefined)
      step()
  }

  function onVisibility(): void {
    if (!document.hidden && toValue(streaming)) {
      clearTimeout(timer)
      step()
    }
  }

  watch([() => toValue(content), () => toValue(streaming)], arrive)
  document.addEventListener('visibilitychange', onVisibility)
  onBeforeUnmount(() => {
    clearTimeout(timer)
    document.removeEventListener('visibilitychange', onVisibility)
  })

  return { shown, steps }
}

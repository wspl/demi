import { onScopeDispose, ref, shallowRef, toValue, watch, type MaybeRefOrGetter } from 'vue'
import type { TextArrival } from '@demicodes/conversation-client'
import { alignShown, arrivedSteps, nextStepEnd, STREAM_PACE, stepInterval, type RevealStep } from '../ui/stream-reveal'

/**
 * The part of a streaming text the reader sees, a step at a time
 * (`STREAM_PACE`), and the steps still fading in. `safe` gives the part of a
 * text that renders as it will stay, without markdown left half open; the
 * steps never pass it. When the stream ends, whatever remains goes on at the
 * same pace until it all shows, so the end brings no burst of text with its
 * own jump of the list. A text that streams in while the tab is hidden shows
 * whole when the tab comes back, since no one watched it arrive.
 *
 * A text already there when the view mounts shows at once, each piece that
 * arrived less than a fade ago (`arrivals`) fading from its arrival: a new
 * reply's first piece fades as every other does, and a view that mounts
 * again mid-stream goes on with each fade where it is, never starting one over.
 */
export function useStreamReveal(
  content: MaybeRefOrGetter<string>,
  streaming: MaybeRefOrGetter<boolean>,
  safe: (text: string) => string,
  arrivals: MaybeRefOrGetter<readonly TextArrival[] | undefined> = undefined,
) {
  const shown = ref(toValue(content))
  const steps = shallowRef<RevealStep[]>(
    toValue(streaming) ? arrivedSteps(toValue(arrivals) ?? [], shown.value.length, performance.now()) : [],
  )
  let timer: ReturnType<typeof setTimeout> | undefined

  /**
   * Shows `text` up to `end`, the part not shown before as a step when `fade`.
   */
  function show(end: number, text: string, fade = true): void {
    const start = alignShown(shown.value, text).length
    const now = performance.now()
    const fading = steps.value.filter((step) => now - step.at < STREAM_PACE.fadeMs && step.end <= start)
    steps.value = fade && end > start ? [...fading, { start, end, at: now }] : fading
    shown.value = text.slice(0, end)
  }

  // Whether the text was streaming when it last changed, and has not all
  // shown since: the end of a stream goes on at the pace it was showing, a text
  // that changes outside one (read whole, edited) shows as it is.
  let live = toValue(streaming)

  function step(): void {
    timer = undefined
    const text = toValue(content)
    const ending = live && !toValue(streaming)
    if (!live && !toValue(streaming)) {
      show(text.length, text, false)
      return
    }
    live = true
    const from = alignShown(shown.value, text).length
    // Once the stream has ended, all of the text is safe to show.
    const ceiling = ending ? text.length : safe(text).length
    if (document.hidden) {
      show(Math.max(from, ceiling), text)
      live = !ending
      return
    }
    if (from >= ceiling) {
      show(from, text)
      live = !ending
      return
    }
    const end = nextStepEnd(text, from, ceiling, safe)
    show(end, text)
    if (end < ceiling)
      timer = setTimeout(step, stepInterval(ceiling - end))
    else
      live = !ending
  }

  // New text steps at once when nothing is waiting, otherwise the running wait
  // takes it, the end of a stream included.
  function arrive(): void {
    if (timer === undefined)
      step()
  }

  function onVisibility(): void {
    if (!document.hidden && live) {
      clearTimeout(timer)
      step()
    }
  }

  watch([() => toValue(content), () => toValue(streaming)], arrive)
  document.addEventListener('visibilitychange', onVisibility)
  onScopeDispose(() => {
    clearTimeout(timer)
    document.removeEventListener('visibilitychange', onVisibility)
  })

  return { shown, steps }
}

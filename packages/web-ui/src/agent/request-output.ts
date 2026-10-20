import { computed, ref, shallowRef, watch, type ComputedRef, type Ref } from 'vue'
import { defaultWindow, transition, tryOnScopeDispose } from '@vueuse/core'
import type { RequestOutputReading } from '@demicodes/conversation-client'

/** How long the count rolls to its next value within a request. */
export const ROLL_MS = 500
/** How long the last reading stays once the turn ended, before it fades. */
export const LINGER_MS = 3_000
/**
 * How long a request streams before its speed shows: over less, a few tokens
 * read as thousands a second.
 */
export const SPEED_AFTER_MS = 1_000
/** How often the speed of a streaming request follows the time. */
const SECOND_MS = 1_000

export interface RequestOutputSources {
  /** The server's reading of the request the root agent runs. */
  output: () => RequestOutputReading | null
  /** The conversation the reading belongs to: another one's shows at once. */
  scope: () => string
  /** The user asked for less motion: the count changes at once. */
  reducedMotion: () => boolean
  /** Where the roll's frames come from; none changes the count at once. */
  window?: Window
}

export interface RequestOutputDisplay {
  /** The reading shown, which stays `LINGER_MS` after the turn ended. */
  shown: Readonly<Ref<RequestOutputReading | null>>
  /** The count as it shows, rolling to the reading's within a request. */
  count: Readonly<Ref<number>>
  /** Tokens a second once the request streamed for `SPEED_AFTER_MS`; null before. */
  speed: ComputedRef<number | null>
}

/**
 * What the chips row shows of the request the root agent runs
 * (`docs/decisions/0012-live-output-tokens.md`): the server's reading,
 * with the count rolling within a request, at once for a new request or
 * another conversation, and the last reading kept a moment after the turn.
 *
 * While a request streams its speed follows a clock that ticks once a second
 * of its streaming, the first tick at its first second; the clock stops at
 * the request's end, and it and the linger are released with the reading
 * they are for or the scope it runs in.
 */
export function useRequestOutput(sources: RequestOutputSources): RequestOutputDisplay {
  const view = sources.window ?? defaultWindow
  const shown = shallowRef<RequestOutputReading | null>(null)
  const count = ref(0)
  // A roll goes on while its number is the latest.
  let roll = 0
  let linger: ReturnType<typeof setTimeout> | undefined
  let tick: ReturnType<typeof setTimeout> | undefined
  // Bumped at each second a streaming request reaches, so its speed follows.
  const seconds = ref(0)

  function stopClock(): void {
    clearTimeout(tick)
    tick = undefined
  }

  /** Ticks at each whole second since `firstTokenAt`. */
  function startClock(firstTokenAt: number): void {
    const next = SECOND_MS - ((Date.now() - firstTokenAt) % SECOND_MS)
    tick = setTimeout(() => {
      seconds.value += 1
      startClock(firstTokenAt)
    }, next)
  }

  function release(): void {
    roll += 1
    clearTimeout(linger)
    linger = undefined
    stopClock()
  }

  function show(reading: RequestOutputReading, rolls: boolean): void {
    release()
    shown.value = reading
    if (rolls && view !== undefined) {
      const current = roll
      // It ends, never fails, when a later change aborts it.
      void transition(count, count.value, reading.tokens, { duration: ROLL_MS, window: view, abort: () => current !== roll })
    } else {
      count.value = reading.tokens
    }
    if (reading.firstTokenAt !== null && reading.endedAt === null)
      startClock(reading.firstTokenAt)
  }

  function hide(): void {
    release()
    shown.value = null
  }

  // Every reading counts, not only the last of a tick: the response's own
  // count comes in the same tick as the turn's end, and is what lingers.
  watch([sources.output, sources.scope], ([output, scope], previous) => {
    const sameScope = previous !== undefined && previous[1] === scope
    if (output !== null) {
      const sameRequest = sameScope && shown.value?.requestId === output.requestId
      show(output, sameRequest && !sources.reducedMotion())
    } else if (sameScope && shown.value !== null) {
      // The turn ended: the last reading stays a moment.
      if (linger === undefined) {
        stopClock()
        linger = setTimeout(hide, LINGER_MS)
      }
    } else {
      hide()
    }
  }, { immediate: true, flush: 'sync' })

  tryOnScopeDispose(release)

  const speed = computed(() => {
    void seconds.value
    const reading = shown.value
    if (reading === null || reading.firstTokenAt === null)
      return null
    const streamed = (reading.endedAt ?? Date.now()) - reading.firstTokenAt
    return streamed < SPEED_AFTER_MS ? null : reading.tokens / (streamed / 1_000)
  })

  return { shown, count, speed }
}

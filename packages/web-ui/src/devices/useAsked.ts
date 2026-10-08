import { computed, nextTick, ref, watch } from 'vue'

/**
 * A button that starts work Demi also starts by itself, as Try Again and
 * Measure on a device's page (`direct-channel.md` § What the user sees):
 * only work the user asked for shows, never work Demi started. A click
 * while none runs asks the host to start it (`start`); a click while Demi's
 * runs starts nothing and adopts it. Either way the button is loading until
 * the work ends.
 *
 * `running` is the host's: whether the work runs, whoever started it. The
 * host sets it as it starts the work, before the page next renders; a click
 * the host starts nothing for (the device went, the browser blocks it)
 * shows nothing.
 */
export function useAsked(running: () => boolean, start: () => void) {
  /** The user asked for the work that runs, or is about to. */
  const asked = ref(false)
  // Synchronous, so work that ends as the next starts is never missed: the
  // next is Demi's again.
  watch(running, (now, was) => {
    if (was && !now)
      asked.value = false
  }, { flush: 'sync' })

  async function click(): Promise<void> {
    if (asked.value)
      return
    asked.value = true
    if (running())
      return
    start()
    await nextTick()
    if (!running())
      asked.value = false
  }

  return { loading: computed(() => asked.value && running()), click }
}

import { computed, nextTick, ref, watch } from 'vue'

/**
 * Try Again on a device's page (`direct-channel.md` § What the user sees):
 * only an attempt the user asked for shows, never one Demi starts by itself.
 * A click while no attempt runs asks the host to start one (`start`); a click
 * while one Demi started runs starts nothing and adopts that one. Either way
 * the button is loading until the attempt ends.
 *
 * `trying` is the host's: whether an attempt runs, whoever started it. The
 * host sets it as it starts one, before the page next renders; a click the
 * host starts no attempt for (the device went, the browser blocks it)
 * shows nothing.
 */
export function useTryAgain(trying: () => boolean, start: () => void) {
  /** The user asked for the attempt that runs, or is about to. */
  const asked = ref(false)
  // Synchronous, so an attempt that ends as the next one starts is never
  // missed: the next one is Demi's again.
  watch(trying, (now, was) => {
    if (was && !now)
      asked.value = false
  }, { flush: 'sync' })

  async function click(): Promise<void> {
    if (asked.value)
      return
    asked.value = true
    if (trying())
      return
    start()
    await nextTick()
    if (!trying())
      asked.value = false
  }

  return { loading: computed(() => asked.value && trying()), click }
}

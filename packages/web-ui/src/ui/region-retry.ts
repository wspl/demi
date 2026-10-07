import { readonly, ref, watch, type Ref } from 'vue'

/** A region's Retry, which shows the region loading until the caller's state takes over (`RegionStatus`). */
export interface RegionRetry {
  /** Whether the region shows its loading pane for a Retry. */
  readonly retrying: Readonly<Ref<boolean>>
  /**
   * Runs `run`, the caller's Retry, with the region loading from now: until
   * the promise `run` returns settles, or, when it returns none, until
   * `state` next changes.
   */
  retry(run: () => unknown): void
}

/** The Retry of a region whose caller's state reads as `state`, in the calling effect scope. */
export function regionRetry(state: () => readonly unknown[]): RegionRetry {
  const retrying = ref(false)
  // The caller's state moved on: it says what the region shows again.
  watch(state, () => {
    retrying.value = false
  })
  return {
    retrying: readonly(retrying),
    retry(run) {
      retrying.value = true
      const ran = run()
      if (ran instanceof Promise) {
        // The caller reports its own failure; the region only stops waiting for it.
        const settled = () => {
          retrying.value = false
        }
        ran.then(settled, settled)
      }
    },
  }
}

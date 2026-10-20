import { ref, watch, type Ref } from 'vue'

/**
 * Whether a surface in a catalog host (`overlayContainerKey`) is a specimen
 * pinned open: it was open as the host first rendered it, and has not been closed
 * since.
 *
 * A pinned surface sits beside others and owns neither the page's layers nor its
 * Escape, since one key could not say which of them it means.
 *
 * A surface the user opens there is a layer of the page like the product's: it
 * joins the overlay store, is the top one, and Escape closes it. That holds for
 * one that opens later, and for one mounted open after the host stands (a context
 * menu keyed to each opening). The host's mount, not the surface's own, tells the
 * two apart: the host's element exists only once it has mounted, after its
 * content first renders.
 *
 * Outside a host (`host` null) nothing is pinned.
 */
export function usePinnedOpen(isOpen: () => boolean, host: Readonly<Ref<object | undefined>> | null): Readonly<Ref<boolean>> {
  const pinned = ref(host !== null && host.value === undefined && isOpen())
  // Sync, so a close and an open in the same tick still end the pin before the surface registers.
  watch(isOpen, (open) => {
    if (!open)
      pinned.value = false
  }, { flush: 'sync' })
  return pinned
}

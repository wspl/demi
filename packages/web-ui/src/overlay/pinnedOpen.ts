import { ref, watch, type Ref } from 'vue'

/**
 * Whether a surface in a catalog host (`overlayContainerKey`) is a specimen
 * pinned open: it was open when it mounted and has not been closed since.
 * A pinned surface sits beside others and owns neither the page's layers nor
 * its Escape, since one key could not say which of them it means. Once the
 * user has closed it and opened it again, or opened one that started closed,
 * it is a layer of the page like the product's: it joins the overlay store,
 * is the top one, and Escape closes it. Outside a host nothing is pinned.
 */
export function usePinnedOpen(isOpen: () => boolean, hosted: boolean): Readonly<Ref<boolean>> {
  const pinned = ref(hosted && isOpen())
  // Sync, so a close and an open in the same tick still end the pin before the surface registers.
  watch(isOpen, (open) => {
    if (!open)
      pinned.value = false
  }, { flush: 'sync' })
  return pinned
}

import { onBeforeUnmount, toValue, watch, type WatchSource } from 'vue'
import { useBackClose } from '../overlay/back-close'
import type { OverlayLayer, OverlayStore, OverlaySurface } from '../overlay/overlayStore'
import { usePhone } from '../ui/phone'

/**
 * Stands a layer in `store` while `isOpen` holds. On a phone a layer the
 * user closes (a menu, a popover, a dialog; not a hint) is also one the
 * browser's Back closes, the topmost first (`useBackClose`), as Android's
 * Back closes what the last tap opened. `backCloses` false leaves Back to a
 * surface that takes it itself, such as the bottom sheet a phone's menu
 * opens in (`Sheet.vue`).
 */
export function useOverlay(
  store: OverlayStore,
  isOpen: WatchSource<boolean>,
  close: () => void,
  layer: OverlayLayer = 'exclusive',
  surface: OverlaySurface = 'popover',
  parent?: string,
  backCloses: WatchSource<boolean> = () => true,
): string {
  const id = crypto.randomUUID()
  let remove = () => {}

  watch(isOpen, (open) => {
    remove()
    if (!open)
      return
    remove = store.push(id, close, layer, surface, parent)
  }, { immediate: true })

  if (layer !== 'hint') {
    const phone = usePhone()
    useBackClose(isOpen, close, () => phone.value && toValue(backCloses))
  }

  onBeforeUnmount(() => {
    remove()
    store.remove(id)
  })

  return id
}

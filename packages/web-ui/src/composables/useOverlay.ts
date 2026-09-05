import { onBeforeUnmount, watch, type WatchSource } from 'vue'
import type { OverlayLayer, OverlayStore } from '../overlay/overlayStore'

export function useOverlay(
  store: OverlayStore,
  isOpen: WatchSource<boolean>,
  close: () => void,
  layer: OverlayLayer = 'exclusive',
): void {
  const id = crypto.randomUUID()
  let remove = () => {}

  watch(isOpen, (open) => {
    remove()
    if (!open) return
    remove = store.push(id, close, layer)
  }, { immediate: true })

  onBeforeUnmount(() => {
    remove()
    store.remove(id)
  })
}

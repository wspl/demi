import { onBeforeUnmount, watch, type WatchSource } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'

export function useOverlay(store: OverlayStore, isOpen: WatchSource<boolean>, close: () => void): void {
  const id = crypto.randomUUID()
  let remove = () => {}

  watch(isOpen, (open) => {
    remove()
    if (!open) return
    remove = store.push(id, close)
  }, { immediate: true })

  onBeforeUnmount(() => {
    remove()
    store.remove(id)
  })
}

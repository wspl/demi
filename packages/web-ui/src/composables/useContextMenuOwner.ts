import { computed, onScopeDispose, ref } from 'vue'

const activeId = ref<symbol | null>(null)
const anchorX = ref(0)
const anchorY = ref(0)
const anchorContextEl = ref<HTMLElement | null>(null)
const menuKey = ref(0)

const closeCallbacks = new Map<symbol, () => void>()

function closeActive() {
  if (activeId.value === null) return
  const cb = closeCallbacks.get(activeId.value)
  activeId.value = null
  cb?.()
}

export function useContextMenuOwner(onClose?: () => void) {
  const id = Symbol()
  if (onClose) closeCallbacks.set(id, onClose)

  const isOpen = computed(() => activeId.value === id)

  function open(event: MouseEvent, contextEl?: HTMLElement | null) {
    closeActive()
    event.preventDefault()
    const el = contextEl
      ?? (event.currentTarget instanceof HTMLElement ? event.currentTarget : null)
    anchorX.value = event.clientX
    anchorY.value = event.clientY
    anchorContextEl.value = el
    menuKey.value++
    activeId.value = id
  }

  function close() {
    if (activeId.value !== id) return
    activeId.value = null
    onClose?.()
  }

  onScopeDispose(() => {
    closeCallbacks.delete(id)
    if (activeId.value === id) activeId.value = null
  })

  return { isOpen, anchorX, anchorY, anchorContextEl, menuKey, open, close }
}

import { onBeforeUnmount, ref, type Ref } from 'vue'
import type { SidebarConversation, SidebarProject, SidebarReorder } from './types'
import { sidebarDrop } from './reorder'

type Entry = Pick<SidebarReorder, 'kind' | 'id'>

/** Pointer sorting keeps the real list in place until drop and supports Escape cancellation. */
export function useSidebarDrag(
  container: Ref<HTMLElement | undefined>,
  projects: () => readonly SidebarProject[],
  conversations: () => readonly SidebarConversation[],
  commit: (request: SidebarReorder) => void,
) {
  const source = ref<Entry | null>(null)
  const target = ref<{ id: string; after: boolean; request: SidebarReorder } | null>(null)
  const pointer = ref({ x: 0, y: 0 })
  let candidate: { entry: Entry; x: number; y: number; pointerId: number } | null = null
  let frame = 0
  let suppressClick = false

  function locate() {
    const element = document
      .elementFromPoint(pointer.value.x, pointer.value.y)
      ?.closest<HTMLElement>('[data-sidebar-id]')
    if (!source.value || !element || !container.value?.contains(element)) {
      target.value = null
      return
    }
    const kind = element.dataset.sidebarKind
    if (kind !== 'project' && kind !== 'conversation') {
      target.value = null
      return
    }
    const id = element.dataset.sidebarId!
    const rect = element.getBoundingClientRect()
    const after = pointer.value.y > rect.top + rect.height / 2
    const request = sidebarDrop(source.value, { kind, id }, after, projects(), conversations())
    target.value = request ? { id, after, request } : null
  }

  function scroll() {
    const element = container.value
    if (!source.value || !element) return
    const rect = element.getBoundingClientRect()
    if (pointer.value.x >= rect.left && pointer.value.x <= rect.right) {
      const edge = 36
      const distance =
        pointer.value.y < rect.top + edge
          ? pointer.value.y - rect.top - edge
          : pointer.value.y > rect.bottom - edge
            ? pointer.value.y - rect.bottom + edge
            : 0
      if (distance) {
        element.scrollTop += Math.max(-10, Math.min(10, distance / 3))
        locate()
      }
    }
    frame = requestAnimationFrame(scroll)
  }

  function cancel() {
    cancelAnimationFrame(frame)
    candidate = null
    source.value = null
    target.value = null
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', finish)
    window.removeEventListener('pointercancel', cancel)
    window.removeEventListener('keydown', keydown, true)
    window.removeEventListener('blur', cancel)
  }

  function move(event: PointerEvent) {
    if (!candidate || event.pointerId !== candidate.pointerId) return
    pointer.value = { x: event.clientX, y: event.clientY }
    if (!source.value) {
      if (Math.hypot(event.clientX - candidate.x, event.clientY - candidate.y) < 5) return
      source.value = candidate.entry
      suppressClick = true
      frame = requestAnimationFrame(scroll)
    }
    event.preventDefault()
    locate()
  }

  function finish(event: PointerEvent) {
    if (!candidate || event.pointerId !== candidate.pointerId) return
    if (source.value) {
      pointer.value = { x: event.clientX, y: event.clientY }
      locate()
      if (target.value) commit(target.value.request)
    }
    cancel()
  }

  function keydown(event: KeyboardEvent) {
    if (event.key !== 'Escape') return
    event.preventDefault()
    event.stopPropagation()
    cancel()
  }

  function start(event: PointerEvent, entry: Entry) {
    if (
      event.button !== 0 ||
      !(event.target instanceof Element) ||
      event.target.closest('input, [aria-label], [data-no-drag]')
    )
      return
    cancel()
    suppressClick = false
    candidate = { entry, x: event.clientX, y: event.clientY, pointerId: event.pointerId }
    window.addEventListener('pointermove', move, { passive: false })
    window.addEventListener('pointerup', finish)
    window.addEventListener('pointercancel', cancel)
    window.addEventListener('keydown', keydown, true)
    window.addEventListener('blur', cancel)
  }

  function click(event: MouseEvent) {
    if (!suppressClick) return
    event.preventDefault()
    event.stopPropagation()
    suppressClick = false
  }

  function pointerDown() {
    suppressClick = false
  }
  onBeforeUnmount(cancel)
  return { source, target, pointer, start, cancel, click, pointerDown }
}

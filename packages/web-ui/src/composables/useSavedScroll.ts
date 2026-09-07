import { nextTick, onBeforeUnmount, watch, type Ref } from 'vue'

interface ScrollPosition {
  top: number
  anchor: string | null
  offset: number
}

/** Restore a scroll viewport by location key, optionally anchored to stable content. */
export function useSavedScroll(viewport: Ref<HTMLElement | undefined>, key: () => string) {
  let stop = () => {}
  const unwatch = watch([viewport, key], async ([el, storageKey], _, onCleanup) => {
    if (!el) return
    let disposed = false
    let restoring = true
    let observer: ResizeObserver | undefined
    let timer: ReturnType<typeof setTimeout> | undefined
    let frame = 0
    let saved: ScrollPosition | null = null
    try {
      const value: unknown = JSON.parse(sessionStorage.getItem(storageKey) ?? 'null')
      if (isScrollPosition(value)) saved = value
    } catch { /* Storage may be unavailable in an embedded browser. */ }

    const anchors = () => [...el.querySelectorAll<HTMLElement>('[data-scroll-anchor]')]
    const relativeTop = (item: HTMLElement) => item.getBoundingClientRect().top - el.getBoundingClientRect().top
    const save = () => {
      if (restoring) return
      const items = anchors()
      const anchor = items.find(item => relativeTop(item) + item.offsetHeight > 0)
      const position: ScrollPosition = {
        top: el.scrollTop,
        anchor: anchor?.dataset.scrollAnchor ?? null,
        offset: anchor ? relativeTop(anchor) : 0,
      }
      try { sessionStorage.setItem(storageKey, JSON.stringify(position)) }
      catch { /* Scrolling must still work when storage is unavailable. */ }
    }
    const restore = () => {
      if (!saved || !restoring) return
      const anchor = anchors().find(item => item.dataset.scrollAnchor === saved!.anchor)
      el.scrollTop = anchor ? el.scrollTop + relativeTop(anchor) - saved.offset : saved.top
    }
    const finish = () => {
      restoring = false
      observer?.disconnect()
      clearTimeout(timer)
    }
    const interact = () => { finish() }
    const leave = () => { if (!restoring) save() }
    el.addEventListener('scroll', save, { passive: true })
    el.addEventListener('wheel', interact, { passive: true })
    el.addEventListener('pointerdown', interact, { passive: true })
    el.addEventListener('keydown', interact)
    window.addEventListener('pagehide', leave)
    stop = () => {
      disposed = true
      // Route cleanup may run after its content has changed; keep the last scroll event's snapshot.
      finish()
      cancelAnimationFrame(frame)
      el.removeEventListener('scroll', save)
      el.removeEventListener('wheel', interact)
      el.removeEventListener('pointerdown', interact)
      el.removeEventListener('keydown', interact)
      window.removeEventListener('pagehide', leave)
    }
    onCleanup(stop)
    await nextTick()
    if (disposed) return
    if (!saved) { el.scrollTop = 0; finish(); return }
    restore()
    frame = requestAnimationFrame(restore)
    // Allow initial fonts and folded content to settle without overwriting the saved position.
    observer = new ResizeObserver(restore)
    for (const child of el.children) observer.observe(child)
    timer = setTimeout(finish, 1500)
  }, { immediate: true, flush: 'pre' })
  onBeforeUnmount(unwatch)
}

function isScrollPosition(value: unknown): value is ScrollPosition {
  if (typeof value !== 'object' || value === null) return false
  const p = value as Record<string, unknown>
  return typeof p.top === 'number' && Number.isFinite(p.top) && p.top >= 0
    && typeof p.offset === 'number' && Number.isFinite(p.offset)
    && (p.anchor === null || typeof p.anchor === 'string')
}

import { nextTick, onBeforeUnmount, watch, type Ref } from 'vue'

interface ScrollPosition {
  top: number
  anchor: string | null
  offset: number
}

/** Restore a scroll viewport by location key, optionally anchored to stable content. */
export function useSavedScroll(
  viewport: Ref<HTMLElement | undefined>,
  key: () => string
) {
  const unwatch = watch([viewport, key], async (
    [viewportElement, storageKey],
    _,
    onCleanup
  ) => {
    if (!viewportElement)
      return
    const element = viewportElement

    const saved = readPosition(storageKey)
    let disposed = false
    let restoring = true
    let observer: ResizeObserver | undefined
    let timer: ReturnType<typeof setTimeout> | undefined
    let frame = 0

    function anchors() {
      return [...element.querySelectorAll<HTMLElement>('[data-scroll-anchor]')]
    }

    function relativeTop(item: HTMLElement) {
      return item.getBoundingClientRect().top - element.getBoundingClientRect().top
    }

    function save() {
      if (restoring)
        return

      const anchor = anchors().find(item => relativeTop(item) + item.offsetHeight > 0)
      writePosition(storageKey, {
        top: element.scrollTop,
        anchor: anchor?.dataset.scrollAnchor ?? null,
        offset: anchor ? relativeTop(anchor) : 0,
      })
    }

    function restore() {
      if (!saved || !restoring)
        return

      const anchor = anchors().find(item => item.dataset.scrollAnchor === saved.anchor)
      element.scrollTop = anchor
        ? element.scrollTop + relativeTop(anchor) - saved.offset
        : saved.top
    }

    function finishRestoration() {
      restoring = false
      observer?.disconnect()
      clearTimeout(timer)
      cancelAnimationFrame(frame)
    }

    element.addEventListener('scroll', save, { passive: true })
    element.addEventListener('wheel', finishRestoration, { passive: true })
    element.addEventListener('pointerdown', finishRestoration, { passive: true })
    element.addEventListener('keydown', finishRestoration)
    window.addEventListener('pagehide', save)

    onCleanup(() => {
      disposed = true
      // Content may already have changed; preserve the last scroll event's snapshot.
      finishRestoration()
      element.removeEventListener('scroll', save)
      element.removeEventListener('wheel', finishRestoration)
      element.removeEventListener('pointerdown', finishRestoration)
      element.removeEventListener('keydown', finishRestoration)
      window.removeEventListener('pagehide', save)
    })

    await nextTick()
    if (disposed || !restoring)
      return

    if (!saved) {
      element.scrollTop = 0
      finishRestoration()
      return
    }

    restore()
    frame = requestAnimationFrame(restore)
    // Allow initial fonts and folded content to settle without overwriting the saved position.
    observer = new ResizeObserver(restore)
    for (const child of element.children) {
      observer.observe(child)
    }
    timer = setTimeout(finishRestoration, 1500)
  }, { immediate: true, flush: 'pre' })

  onBeforeUnmount(unwatch)
}

function readPosition(key: string): ScrollPosition | null {
  try {
    const value: unknown = JSON.parse(sessionStorage.getItem(key) ?? 'null')
    return isScrollPosition(value) ? value : null
  } catch {
    // Storage may be unavailable, or contain an invalid snapshot; start at the top.
    return null
  }
}

function writePosition(key: string, position: ScrollPosition): void {
  try {
    sessionStorage.setItem(key, JSON.stringify(position))
  } catch {
    // Scrolling must still work when storage is unavailable.
  }
}

function isScrollPosition(value: unknown): value is ScrollPosition {
  if (typeof value !== 'object' || value === null)
    return false

  const position = value as Record<string, unknown>
  return typeof position.top === 'number'
    && Number.isFinite(position.top)
    && position.top >= 0
    && typeof position.offset === 'number'
    && Number.isFinite(position.offset)
    && (position.anchor === null || typeof position.anchor === 'string')
}

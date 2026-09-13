import type { EditorView } from '@codemirror/view'
import { computeScrollbarAxis } from './metrics'
import type { RawScrollbarMarker } from './markers'

export interface MountedScrollbarDom {
  vertical: HTMLDivElement
  horizontal: HTMLDivElement
  verticalThumb: HTMLDivElement
  horizontalThumb: HTMLDivElement
  update(view: EditorView, markers?: RawScrollbarMarker[]): void
  setScrollActivity(activity: { vertical?: boolean; horizontal?: boolean }): void
  destroy(): void
}

export interface MountedScrollbarDomOptions {
  splitDiffLanes?: boolean
}

export function mountScrollbarDom(view: EditorView, options: MountedScrollbarDomOptions = {}): MountedScrollbarDom {
  const idleMs = 900
  const vertical = document.createElement('div')
  vertical.dataset['editorScrollbar'] = 'vertical'
  vertical.dataset['scrollbarVisible'] = 'false'
  vertical.dataset['scrollbarScrollable'] = 'false'
  const verticalMarkers = document.createElement('div')
  verticalMarkers.dataset['scrollbarMarkers'] = options.splitDiffLanes ? 'vertical-split' : 'vertical'
  const verticalLeftMarkers = document.createElement('div')
  verticalLeftMarkers.dataset['scrollbarMarkers'] = 'vertical-left'
  const verticalRightMarkers = document.createElement('div')
  verticalRightMarkers.dataset['scrollbarMarkers'] = 'vertical-right'
  if (options.splitDiffLanes) {
    verticalMarkers.appendChild(verticalLeftMarkers)
    verticalMarkers.appendChild(verticalRightMarkers)
  }
  const verticalThumb = document.createElement('div')
  verticalThumb.dataset['scrollbarThumb'] = 'vertical'
  vertical.appendChild(verticalMarkers)
  vertical.appendChild(verticalThumb)

  const horizontal = document.createElement('div')
  horizontal.dataset['editorScrollbar'] = 'horizontal'
  horizontal.dataset['scrollbarVisible'] = 'false'
  horizontal.dataset['scrollbarScrollable'] = 'false'
  const horizontalThumb = document.createElement('div')
  horizontalThumb.dataset['scrollbarThumb'] = 'horizontal'
  horizontal.appendChild(horizontalThumb)

  view.dom.appendChild(vertical)
  view.dom.appendChild(horizontal)

  let verticalDragCleanup: (() => void) | undefined
  let horizontalDragCleanup: (() => void) | undefined
  let verticalScrollTimer = 0
  let horizontalScrollTimer = 0
  let verticalHovered = false
  let horizontalHovered = false
  let verticalDragging = false
  let horizontalDragging = false

  const syncVisibility = (track: HTMLDivElement, scrollable: boolean, active: boolean) => {
    track.dataset['scrollbarVisible'] = scrollable && active ? 'true' : 'false'
    track.dataset['scrollbarScrollable'] = scrollable ? 'true' : 'false'
  }

  const refreshVisibility = (editorView: EditorView) => {
    const verticalScrollable = editorView.scrollDOM.scrollHeight > editorView.scrollDOM.clientHeight
    const horizontalScrollable = editorView.scrollDOM.scrollWidth > editorView.scrollDOM.clientWidth
    syncVisibility(vertical, verticalScrollable, verticalHovered || verticalDragging || verticalScrollTimer > 0)
    syncVisibility(horizontal, horizontalScrollable, horizontalHovered || horizontalDragging || horizontalScrollTimer > 0)
  }

  const clearTimer = (timer: number) => {
    if (timer > 0) window.clearTimeout(timer)
  }

  const bumpScrollActivity = (axis: 'vertical' | 'horizontal', editorView: EditorView) => {
    if (axis === 'vertical') {
      clearTimer(verticalScrollTimer)
      verticalScrollTimer = window.setTimeout(() => {
        verticalScrollTimer = 0
        refreshVisibility(editorView)
      }, idleMs)
    } else {
      clearTimer(horizontalScrollTimer)
      horizontalScrollTimer = window.setTimeout(() => {
        horizontalScrollTimer = 0
        refreshVisibility(editorView)
      }, idleMs)
    }
    refreshVisibility(editorView)
  }

  const renderMarkerNode = (marker: RawScrollbarMarker, parent: HTMLDivElement) => {
    const node = document.createElement('div')
    node.dataset['editorScrollbarMarker'] = marker.kind
    if (marker.severity) {
      node.dataset['editorScrollbarSeverity'] = marker.severity
    }
    node.dataset['editorScrollbarPriority'] = String(marker.priority)
    node.style.position = 'absolute'
    node.style.left = '0'
    node.style.right = '0'
    node.style.top = `${marker.startRatio * 100}%`
    const lineCount = Math.max(1, Math.round((marker.endRatio - marker.startRatio) * view.state.doc.lines))
    if (marker.kind === 'diagnostic' || marker.kind === 'search') {
      node.style.height = `${lineCount * 2}px`
    } else {
      node.style.height = `${(marker.endRatio - marker.startRatio) * 100}%`
    }
    node.style.zIndex = String(marker.priority)
    parent.appendChild(node)
  }

  const updateMarkers = (markers: RawScrollbarMarker[]) => {
    if (!options.splitDiffLanes) {
      verticalMarkers.replaceChildren()
      for (const marker of markers) {
        renderMarkerNode(marker, verticalMarkers)
      }
      return
    }

    verticalLeftMarkers.replaceChildren()
    verticalRightMarkers.replaceChildren()
    for (const marker of markers) {
      const lane = marker.kind === 'diff-added' || marker.kind === 'diff-deleted'
        ? verticalLeftMarkers
        : verticalRightMarkers
      renderMarkerNode(marker, lane)
    }
  }

  const update = (editorView: EditorView, markers: RawScrollbarMarker[] = []) => {
    const verticalMetrics = computeScrollbarAxis({
      viewportSize: editorView.scrollDOM.clientHeight,
      scrollSize: editorView.scrollDOM.scrollHeight,
      scrollOffset: editorView.scrollDOM.scrollTop,
      trackSize: editorView.dom.clientHeight,
      minThumbSize: 18,
    })

    verticalThumb.style.height = `${verticalMetrics.thumbSize}px`
    verticalThumb.style.transform = `translateY(${verticalMetrics.thumbOffset}px)`
    vertical.style.pointerEvents = verticalMetrics.scrollable ? 'auto' : 'none'

    const horizontalMetrics = computeScrollbarAxis({
      viewportSize: editorView.scrollDOM.clientWidth,
      scrollSize: editorView.scrollDOM.scrollWidth,
      scrollOffset: editorView.scrollDOM.scrollLeft,
      trackSize: editorView.dom.clientWidth,
      minThumbSize: 18,
    })

    horizontalThumb.style.width = `${horizontalMetrics.thumbSize}px`
    horizontalThumb.style.transform = `translateX(${horizontalMetrics.thumbOffset}px)`
    horizontal.style.pointerEvents = horizontalMetrics.scrollable ? 'auto' : 'none'
    updateMarkers(markers)
    refreshVisibility(editorView)
  }

  const syncVerticalScroll = (ratio: number) => {
    const maxScroll = view.scrollDOM.scrollHeight - view.scrollDOM.clientHeight
    view.scrollDOM.scrollTop = Math.max(0, Math.min(maxScroll, ratio * maxScroll))
    update(view)
  }

  const syncHorizontalScroll = (ratio: number) => {
    const maxScroll = view.scrollDOM.scrollWidth - view.scrollDOM.clientWidth
    view.scrollDOM.scrollLeft = Math.max(0, Math.min(maxScroll, ratio * maxScroll))
    update(view)
  }

  vertical.addEventListener('pointerdown', (event) => {
    if (event.target === verticalThumb) return
    const rect = vertical.getBoundingClientRect()
    if (rect.height <= 0) return
    const ratio = (event.clientY - rect.top) / rect.height
    syncVerticalScroll(ratio)
    bumpScrollActivity('vertical', view)
  })

  vertical.addEventListener('pointerenter', () => {
    verticalHovered = true
    refreshVisibility(view)
  })
  vertical.addEventListener('pointerleave', () => {
    verticalHovered = false
    refreshVisibility(view)
  })

  verticalThumb.addEventListener('pointerdown', (event) => {
    event.stopPropagation()
    verticalDragging = true
    refreshVisibility(view)
    const trackRect = vertical.getBoundingClientRect()
    const thumbRect = verticalThumb.getBoundingClientRect()
    const startY = event.clientY
    const startTop = thumbRect.top - trackRect.top
    const maxThumbOffset = Math.max(0, trackRect.height - thumbRect.height)

    const handleMove = (moveEvent: PointerEvent) => {
      if (maxThumbOffset <= 0) return
      const nextOffset = Math.max(0, Math.min(maxThumbOffset, startTop + (moveEvent.clientY - startY)))
      syncVerticalScroll(nextOffset / maxThumbOffset)
    }

    const handleUp = () => {
      verticalDragging = false
      verticalDragCleanup?.()
      verticalDragCleanup = undefined
      bumpScrollActivity('vertical', view)
    }

    window.addEventListener('pointermove', handleMove)
    window.addEventListener('pointerup', handleUp, { once: true })

    verticalDragCleanup = () => {
      window.removeEventListener('pointermove', handleMove)
      window.removeEventListener('pointerup', handleUp)
    }
  })

  horizontal.addEventListener('pointerdown', (event) => {
    if (event.target === horizontalThumb) return
    const rect = horizontal.getBoundingClientRect()
    if (rect.width <= 0) return
    const ratio = (event.clientX - rect.left) / rect.width
    syncHorizontalScroll(ratio)
    bumpScrollActivity('horizontal', view)
  })

  horizontal.addEventListener('pointerenter', () => {
    horizontalHovered = true
    refreshVisibility(view)
  })
  horizontal.addEventListener('pointerleave', () => {
    horizontalHovered = false
    refreshVisibility(view)
  })

  horizontalThumb.addEventListener('pointerdown', (event) => {
    event.stopPropagation()
    horizontalDragging = true
    refreshVisibility(view)
    const trackRect = horizontal.getBoundingClientRect()
    const thumbRect = horizontalThumb.getBoundingClientRect()
    const startX = event.clientX
    const startLeft = thumbRect.left - trackRect.left
    const maxThumbOffset = Math.max(0, trackRect.width - thumbRect.width)

    const handleMove = (moveEvent: PointerEvent) => {
      if (maxThumbOffset <= 0) return
      const nextOffset = Math.max(0, Math.min(maxThumbOffset, startLeft + (moveEvent.clientX - startX)))
      syncHorizontalScroll(nextOffset / maxThumbOffset)
    }

    const handleUp = () => {
      horizontalDragging = false
      horizontalDragCleanup?.()
      horizontalDragCleanup = undefined
      bumpScrollActivity('horizontal', view)
    }

    window.addEventListener('pointermove', handleMove)
    window.addEventListener('pointerup', handleUp, { once: true })

    horizontalDragCleanup = () => {
      window.removeEventListener('pointermove', handleMove)
      window.removeEventListener('pointerup', handleUp)
    }
  })

  return {
    vertical,
    horizontal,
    verticalThumb,
    horizontalThumb,
    update,
    setScrollActivity(activity) {
      if (activity.vertical) bumpScrollActivity('vertical', view)
      if (activity.horizontal) bumpScrollActivity('horizontal', view)
    },
    destroy() {
      clearTimer(verticalScrollTimer)
      clearTimer(horizontalScrollTimer)
      verticalDragCleanup?.()
      horizontalDragCleanup?.()
      vertical.remove()
      horizontal.remove()
    },
  }
}

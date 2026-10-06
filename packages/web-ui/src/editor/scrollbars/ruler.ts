import type { EditorView } from '@codemirror/view'
import type { RawScrollbarMarker } from './markers'

/**
 * The marks along the editor's right edge, under its vertical scrollbar:
 * where the selection, the find bar's matches and a diff's changes are in the
 * whole text. A diff view gives its changes a lane of their own, left of the
 * other marks. The marks show while the text can scroll.
 */
export interface ScrollbarRuler {
  update(view: EditorView, markers: RawScrollbarMarker[]): void
  destroy(): void
}

export interface ScrollbarRulerOptions {
  splitDiffLanes: boolean
}

export function mountScrollbarRuler(view: EditorView, options: ScrollbarRulerOptions): ScrollbarRuler {
  const ruler = document.createElement('div')
  ruler.dataset['scrollbarMarkers'] = options.splitDiffLanes ? 'vertical-split' : 'vertical'
  ruler.dataset['scrollbarScrollable'] = 'false'
  const leftLane = document.createElement('div')
  leftLane.dataset['scrollbarMarkers'] = 'vertical-left'
  const rightLane = document.createElement('div')
  rightLane.dataset['scrollbarMarkers'] = 'vertical-right'
  if (options.splitDiffLanes) {
    ruler.appendChild(leftLane)
    ruler.appendChild(rightLane)
  }
  view.dom.appendChild(ruler)

  const renderMarker = (marker: RawScrollbarMarker, lane: HTMLDivElement) => {
    const node = document.createElement('div')
    node.dataset['editorScrollbarMarker'] = marker.kind
    node.style.position = 'absolute'
    node.style.left = '0'
    node.style.right = '0'
    node.style.top = `${marker.startRatio * 100}%`
    const lineCount = Math.max(1, Math.round((marker.endRatio - marker.startRatio) * view.state.doc.lines))
    if (marker.kind === 'search') {
      node.style.height = `${lineCount * 2}px`
    } else {
      node.style.height = `${(marker.endRatio - marker.startRatio) * 100}%`
    }
    node.style.zIndex = String(marker.priority)
    lane.appendChild(node)
  }

  const laneOf = (marker: RawScrollbarMarker) => {
    if (!options.splitDiffLanes)
      return ruler
    return marker.kind === 'diff-added' || marker.kind === 'diff-deleted' ? leftLane : rightLane
  }

  return {
    update(editorView, markers) {
      const scrollable = editorView.scrollDOM.scrollHeight > editorView.scrollDOM.clientHeight
      ruler.dataset['scrollbarScrollable'] = scrollable ? 'true' : 'false'
      if (options.splitDiffLanes) {
        leftLane.replaceChildren()
        rightLane.replaceChildren()
      } else {
        ruler.replaceChildren()
      }
      for (const marker of markers)
        renderMarker(marker, laneOf(marker))
    },
    destroy() {
      ruler.remove()
    },
  }
}

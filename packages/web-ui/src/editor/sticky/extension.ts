import type { Extension } from '@codemirror/state'
import { EditorView, ViewPlugin, type ViewUpdate } from '@codemirror/view'
import { createStickyOverlayDom } from './dom'
import { createStickyHeaderViews } from './headerViews'
import { getStickyStructureStack, type StickyStructureItem } from './structure'

const STICKY_HEADER_EPSILON_PX = 0.01

export function stickyHeadersExtension(): Extension {
  return ViewPlugin.fromClass(class {
    overlay: ReturnType<typeof createStickyOverlayDom>
    headers: ReturnType<typeof createStickyHeaderViews>
    private syncScheduled = false
    private applyFrame = 0
    private pendingMeasure:
      | {
          items: StickyStructureItem[]
          scrollLeft: number
          gutterWidths: { lineNumbers: number; fold: number }
          frame: { top: number; left: number; width: number; paddingTop: number }
        }
      | null = null

    private readonly onScroll = () => {
      this.scheduleSync()
    }

    private getMeasuredStickyRowHeight(fallback: number) {
      const visibleRow = [...this.overlay.rowsHost.querySelectorAll<HTMLElement>('.cm-stickyHeaderRow')]
        .find((row) => getComputedStyle(row).display !== 'none')
      return visibleRow?.getBoundingClientRect().height || fallback
    }

    private getVisibleStickyRowHeights() {
      return [...this.overlay.rowsHost.querySelectorAll<HTMLElement>('.cm-stickyHeaderRow')]
        .filter((row) => getComputedStyle(row).display !== 'none')
        .map((row) => row.getBoundingClientRect().height)
    }

    private getRenderedLineRect(view: EditorView, pos: number) {
      try {
        const domAtPos = view.domAtPos(pos)
        const element = domAtPos.node instanceof Element
          ? domAtPos.node
          : domAtPos.node.parentElement
        const line = element?.closest<HTMLElement>('.cm-line')
        return line?.getBoundingClientRect() ?? null
      }
      catch {
        return null
      }
    }

    view: EditorView
    constructor(view: EditorView) {
      this.view = view
      this.overlay = createStickyOverlayDom(this.view.dom)
      this.headers = createStickyHeaderViews(this.view, this.overlay.rowsHost)
      this.view.scrollDOM.addEventListener('scroll', this.onScroll, { passive: true })
      this.scheduleSync()
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged || update.geometryChanged) {
        this.scheduleSync()
      }
    }

    destroy() {
      this.view.scrollDOM.removeEventListener('scroll', this.onScroll)
      if (this.applyFrame) cancelAnimationFrame(this.applyFrame)
      this.headers.destroy()
      this.overlay.root.remove()
    }

    private scheduleSync() {
      if (this.syncScheduled) return
      this.syncScheduled = true
      this.view.requestMeasure({
        read: (view) => {
          const visibleTop = view.scrollDOM.scrollTop

          // Fast path: when scroll is near the top, no header line can have
          // left the viewport — skip all expensive DOM measurements.
          if (visibleTop < view.defaultLineHeight * 2) {
            return {
              items: [],
              scrollLeft: view.scrollDOM.scrollLeft,
              gutterWidths: { lineNumbers: 0, fold: 0 },
              frame: { top: 0, left: 0, width: 0, paddingTop: 0 },
            }
          }

          const contentPaddingTop = Number.parseFloat(getComputedStyle(view.contentDOM).paddingTop || '0') || 0
          const visibleContentTop = visibleTop + contentPaddingTop
          const topBlock = view.lineBlockAtHeight(visibleContentTop)
          const fallbackStickyRowHeight = Math.ceil(Math.max(view.defaultLineHeight, topBlock.height)) + 1
          const stickyRowHeight = this.getMeasuredStickyRowHeight(fallbackStickyRowHeight)
          const visibleStickyRowHeights = this.getVisibleStickyRowHeights()
          const scrollViewportTop = view.scrollDOM.getBoundingClientRect().top
          const contentProbeX = view.contentDOM.getBoundingClientRect().left + 1

          let items: StickyStructureItem[] = []

          for (let iteration = 0; iteration < 4; iteration += 1) {
            const measuredOffset = visibleStickyRowHeights
              .slice(0, items.length)
              .reduce((sum, height) => sum + height, 0)
            const fallbackOffset = Math.max(0, items.length - visibleStickyRowHeights.length) * stickyRowHeight
            const boundaryOffset = measuredOffset + fallbackOffset
            const boundaryProbeTop = visibleContentTop + boundaryOffset
            const boundaryViewportTop = scrollViewportTop + contentPaddingTop + boundaryOffset
            const beforeProbeTop = Math.max(0, boundaryProbeTop - 0.5)
            const afterProbeTop = boundaryProbeTop + 1
            const beforeProbeBlock = view.lineBlockAtHeight(beforeProbeTop)
            const afterProbeBlock = view.lineBlockAtHeight(afterProbeTop)
            const beforeProbePos = view.posAtCoords({ x: contentProbeX, y: boundaryViewportTop - 0.5 }) ?? beforeProbeBlock.from
            const afterProbePos = view.posAtCoords({ x: contentProbeX, y: boundaryViewportTop + 1 }) ?? afterProbeBlock.from
            const beforeCandidates = getStickyStructureStack(view.state, beforeProbePos)
            const afterCandidates = getStickyStructureStack(view.state, afterProbePos)
            const candidates = [...items, ...beforeCandidates, ...afterCandidates]
              .filter((candidate, index, all) => index === all.findIndex((item) => item.headerLineNumber === candidate.headerLineNumber))
              .sort((a, b) => (a.from - b.from) || (b.to - a.to))
            let stickyOffset = 0
            const nextItems = candidates.filter((item) => {
              const headerLine = view.state.doc.line(item.headerLineNumber)
              const lastPos = Math.max(item.from, item.to - 1)
              const lastLine = view.state.doc.lineAt(lastPos)
              const headerBlock = view.lineBlockAt(headerLine.from)
              const lineBlock = view.lineBlockAt(lastLine.from)
              const boundaryTop = scrollViewportTop + contentPaddingTop + stickyOffset
              const headerLineRect = this.getRenderedLineRect(view, headerLine.from)
              const lastLineRect = this.getRenderedLineRect(view, lastLine.from)
              const headerViewportTop = headerLineRect?.top ?? (scrollViewportTop + contentPaddingTop + headerBlock.top - visibleTop)
              const tailViewportBottom = lastLineRect?.bottom ?? (scrollViewportTop + contentPaddingTop + lineBlock.top + lineBlock.height - visibleTop)
              const headerCrossed = headerViewportTop <= boundaryTop + STICKY_HEADER_EPSILON_PX
              const tailStillVisible = tailViewportBottom >= boundaryTop
              const shouldStick = headerCrossed && tailStillVisible
              if (shouldStick) stickyOffset += stickyRowHeight
              return shouldStick
            })

            const stable = nextItems.length === items.length
              && nextItems.every((item, index) => item.headerLineNumber === items[index]?.headerLineNumber)
            items = nextItems
            if (stable) break
          }

          const lineNumbers = view.dom.querySelector<HTMLElement>('.cm-lineNumbers')
          const foldGutter = view.dom.querySelector<HTMLElement>('.cm-foldGutter')
          const scrollerRect = view.scrollDOM.getBoundingClientRect()
          const editorRect = view.dom.getBoundingClientRect()

          return {
            items,
            scrollLeft: view.scrollDOM.scrollLeft,
            gutterWidths: {
              lineNumbers: lineNumbers?.getBoundingClientRect().width ?? 0,
              fold: foldGutter?.getBoundingClientRect().width ?? 0,
            },
            frame: {
              top: scrollerRect.top - editorRect.top,
              left: scrollerRect.left - editorRect.left,
              width: scrollerRect.width,
              paddingTop: contentPaddingTop,
            },
          }
        },
        write: (measure) => {
          this.syncScheduled = false
          this.pendingMeasure = measure
          if (this.applyFrame) return
          this.applyFrame = requestAnimationFrame(() => {
            this.applyFrame = 0
            const next = this.pendingMeasure
            this.pendingMeasure = null
            if (!next) return

            if (next.items.length === 0) {
              this.headers.render([])
              this.overlay.setReservedHeight(0)
              return
            }

            const items = next.items.map((item) => ({
              text: this.view.state.doc.line(item.headerLineNumber).text,
              lineNumber: item.headerLineNumber,
            }))

            this.overlay.setFrame(next.frame)
            this.overlay.setGutterWidths(next.gutterWidths)
            this.headers.render(items, next.gutterWidths)
            this.overlay.setReservedHeight(this.overlay.rowsHost.offsetHeight + 1)
            this.overlay.setHorizontalOffset(next.scrollLeft)
          })
        },
      })
    }
  })
}

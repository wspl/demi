export interface StickyOverlayDom {
  root: HTMLDivElement
  topPad: HTMLDivElement
  rowsHost: HTMLDivElement
  setFrame(frame: { top: number; left: number; width: number; paddingTop: number }): void
  setGutterWidths(widths: { lineNumbers: number; fold: number }): void
  setHorizontalOffset(offset: number): void
  setReservedHeight(height: number): void
}

/**
 * The overlay lives inside the editor (`host`, the positioned `.cm-editor`),
 * placed relative to it, so it follows the editor wherever the page scrolls
 * or moves it; on the body it stayed where it was last measured.
 */
export function createStickyOverlayDom(host: HTMLElement): StickyOverlayDom {
  const rowsHost = document.createElement('div')
  rowsHost.className = 'cm-stickyHeadersRows'
  rowsHost.style.display = 'flex'
  rowsHost.style.flexDirection = 'column'

  const topPadGutter = document.createElement('div')
  topPadGutter.className = 'cm-stickyHeadersTopPadGutter'
  topPadGutter.style.backgroundColor = 'var(--color-surface)'
  topPadGutter.style.flex = '0 0 auto'

  const topPadContent = document.createElement('div')
  topPadContent.className = 'cm-stickyHeadersTopPadContent'
  topPadContent.style.backgroundColor = 'var(--color-surface-editor)'
  topPadContent.style.flex = '1 1 auto'

  const topPad = document.createElement('div')
  topPad.className = 'cm-stickyHeadersTopPad'
  topPad.style.display = 'flex'
  topPad.style.flex = '0 0 auto'
  topPad.append(topPadGutter, topPadContent)

  const root = document.createElement('div')
  root.className = 'cm-stickyHeaders'
  root.style.position = 'absolute'
  root.style.top = '0'
  root.style.left = '0'
  root.style.width = '0'
  root.style.zIndex = '3'
  root.style.pointerEvents = 'none'
  root.style.overflow = 'hidden'
  root.style.overflowAnchor = 'none'
  root.style.contain = 'layout paint style'
  root.style.backgroundColor = 'transparent'
  root.append(topPad, rowsHost)
  host.append(root)

  return {
    root,
    topPad,
    rowsHost,
    setFrame(frame) {
      root.style.top = `${frame.top}px`
      root.style.left = `${frame.left}px`
      root.style.width = `${frame.width}px`
      topPad.style.height = `${frame.paddingTop}px`
    },
    setGutterWidths(widths) {
      const gutterWidth = widths.lineNumbers + widths.fold
      topPadGutter.style.width = `${gutterWidth}px`
      topPadGutter.style.minWidth = `${gutterWidth}px`
    },
    setHorizontalOffset(offset: number) {
      rowsHost.style.transform = `translateX(${-offset}px)`
    },
    setReservedHeight(height: number) {
      if (height > 0) {
        root.style.removeProperty('height')
      } else {
        root.style.height = '0px'
      }
    },
  }
}

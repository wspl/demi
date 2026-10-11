let measuring: CanvasRenderingContext2D | null | undefined

/**
 * How wide `text` is drawn in `font` (a CSS `font` value, as
 * `getComputedStyle(element).font` gives it), by one canvas every caller
 * shares; 0 where the page has no 2D context to measure with.
 */
export function textWidth(text: string, font: string): number {
  if (measuring === undefined) {
    measuring = document.createElement('canvas').getContext('2d')
  }
  if (!measuring) {
    return 0
  }
  measuring.font = font
  return measuring.measureText(text).width
}

/** Which axis a handle moves along: a vertical divider sizes a width, a horizontal one a height. */
export type ResizeOrientation = 'vertical' | 'horizontal'

/** Where the sized pane sits relative to the handle: before it (start) or after it (end). */
export type ResizeSide = 'start' | 'end'

export interface ResizeBounds {
  min: number
  max: number
}

export function clampSize(value: number, bounds: ResizeBounds): number {
  return Math.min(bounds.max, Math.max(bounds.min, Math.round(value)))
}

/**
 * The size a drag has reached: the size at pointer-down plus how far the
 * pointer moved along the axis, in the pane's direction. A pane after the
 * handle grows when the pointer moves towards the start.
 */
export function sizeFromDrag(
  startSize: number,
  pointerDelta: number,
  side: ResizeSide,
  bounds: ResizeBounds,
): number {
  const direction = side === 'start' ? 1 : -1
  return clampSize(startSize + pointerDelta * direction, bounds)
}

export interface KeyResizeOptions extends ResizeBounds {
  orientation: ResizeOrientation
  side: ResizeSide
  /** One arrow press, in px; Shift multiplies it by four. */
  step: number
}

/**
 * The size a key asks for, or null when the key is not one of the handle's.
 * The arrows along the axis move the edge the way a drag would: towards the
 * end grows a pane before the handle and shrinks one after it. Home and End
 * go to the bounds.
 */
export function sizeFromKey(
  key: string,
  shift: boolean,
  current: number,
  options: KeyResizeOptions,
): number | null {
  const [towardsStart, towardsEnd] = options.orientation === 'vertical'
    ? ['ArrowLeft', 'ArrowRight']
    : ['ArrowUp', 'ArrowDown']
  const step = options.step * (shift ? 4 : 1)
  if (key === towardsEnd) {
    return sizeFromDrag(current, step, options.side, options)
  }
  if (key === towardsStart) {
    return sizeFromDrag(current, -step, options.side, options)
  }
  if (key === 'Home') {
    return options.min
  }
  if (key === 'End') {
    return options.max
  }
  return null
}

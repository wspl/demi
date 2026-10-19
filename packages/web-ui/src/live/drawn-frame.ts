/**
 * A device drawn as a black silhouette around its screen (`mobile.md`
 * § The live view), in the look of the frames Xcode installs: the frame a
 * view draws itself when the device's own is not installed. Its body runs
 * an even bezel around the screen, its outer corners concentric with the
 * screen's, a metal band with a lit edge along its outside, and each button
 * is a short bar standing out of the side it is on. The screen's own shape, the same corners, clips its picture. Sizes
 * are CSS pixels of the view.
 */

/** The bezel's width, as a share of the screen's shorter side. */
const BEZEL = 0.04
/** How far a button stands out of the body, as a share of the bezel. */
const MARK = 0.3
/**
 * The metal band along the body's outer edge, as a share of the bezel, and
 * the width of its lit edge in CSS pixels, as Xcode's iPhone frames draw
 * them: about 4 of a 16-pixel bezel, and its outermost pixel and a half.
 */
const BAND = 0.25
const EDGE = 1.5

/** The bezel around a screen of `screen`. */
export function drawnFrameBezel(screen: { width: number; height: number }): number {
  return BEZEL * Math.min(screen.width, screen.height)
}

/** The room the silhouette takes around a screen, buttons included, as a share of the screen's shorter side. */
export const DRAWN_FRAME_ROOM = BEZEL * (1 + MARK)

export interface FrameBox {
  left: number
  top: number
  width: number
  height: number
}

/** A button on an edge of the device, as fractions of the edge's length from its top or its left. */
export interface FrameButton {
  side: 'left' | 'right' | 'top' | 'bottom'
  start: number
  length: number
}

/** A point of a corner's shape, from the corner of the screen's box: `x` across from its upright edge, `y` down from its level one. */
export interface CornerPoint {
  x: number
  y: number
}

/**
 * One corner of the screen, drawn as if it were the top left one: a quarter
 * circle, square at a radius of 0, or the screen's edge across the corner
 * through `points`, from where it leaves the upright edge to where it meets
 * the level one.
 */
export type ScreenCorner =
  | { kind: 'round'; radius: number }
  | { kind: 'curve'; points: CornerPoint[] }

export interface ScreenCorners {
  topLeft: ScreenCorner
  topRight: ScreenCorner
  bottomRight: ScreenCorner
  bottomLeft: ScreenCorner
}

export const SQUARE_CORNERS: ScreenCorners = {
  topLeft: { kind: 'round', radius: 0 },
  topRight: { kind: 'round', radius: 0 },
  bottomRight: { kind: 'round', radius: 0 },
  bottomLeft: { kind: 'round', radius: 0 },
}

export interface DrawnFrameShape {
  /** The bezel's width. */
  bezel: number
  /** The body, an SVG path in the area's coordinates. */
  body: string
  /** The metal band along the body's outer edge: a line through its middle, and its width. */
  band: string
  bandWidth: number
  /** The band's lit outer edge, a line just inside the body's, and its width. */
  edge: string
  edgeWidth: number
  /** The buttons, each with the side it stands out of. */
  marks: (FrameBox & { side: FrameButton['side'] })[]
}

/** `corners` at `scale`, as when the screen is shown at another size. */
export function scaledCorners(corners: ScreenCorners, scale: number): ScreenCorners {
  const scaled = (corner: ScreenCorner): ScreenCorner => corner.kind === 'round'
    ? { kind: 'round', radius: corner.radius * scale }
    : { kind: 'curve', points: corner.points.map((point) => ({ x: point.x * scale, y: point.y * scale })) }
  return {
    topLeft: scaled(corners.topLeft),
    topRight: scaled(corners.topRight),
    bottomRight: scaled(corners.bottomRight),
    bottomLeft: scaled(corners.bottomLeft),
  }
}

/** A corner's points as the top left one, `offset` further out at an even distance, and the radius of a round one. */
function grown(corner: ScreenCorner, offset: number): { points: CornerPoint[]; radius: number } {
  if (corner.kind === 'round') {
    if (corner.radius <= 0) {
      return { points: [{ x: -offset, y: -offset }], radius: 0 }
    }
    return { points: [{ x: -offset, y: corner.radius }, { x: corner.radius, y: -offset }], radius: corner.radius + offset }
  }
  const points = corner.points
  // Each point moves out along its normal: the upright edge's at the
  // first, the level edge's at the last, between them the mean of the two
  // segments' normals, which point away from the screen.
  const normal = (from: CornerPoint, to: CornerPoint) => {
    const length = Math.hypot(to.x - from.x, to.y - from.y) || 1
    return { x: (to.y - from.y) / length, y: -(to.x - from.x) / length }
  }
  return {
    points: points.map((point, index) => {
      const before = index > 0 ? normal(points[index - 1]!, point) : { x: -1, y: 0 }
      const after = index < points.length - 1 ? normal(point, points[index + 1]!) : { x: 0, y: -1 }
      const sum = { x: before.x + after.x, y: before.y + after.y }
      const length = Math.hypot(sum.x, sum.y) || 1
      return { x: point.x + (sum.x / length) * offset, y: point.y + (sum.y / length) * offset }
    }),
    radius: 0,
  }
}

const number = (value: number) => Number(value.toFixed(2))

/**
 * The SVG path of a `width` × `height` screen at (`left`, `top`) with
 * `corners`, grown by `offset` all round at an even distance, clockwise from
 * the top left corner.
 */
export function screenPath(box: FrameBox, corners: ScreenCorners, offset = 0): string {
  const { left, top, width, height } = box
  // Each corner mirrored into its place, its points in clockwise order.
  const placed: [ScreenCorner, (point: CornerPoint) => CornerPoint, boolean][] = [
    [corners.topLeft, ({ x, y }) => ({ x: left + x, y: top + y }), false],
    [corners.topRight, ({ x, y }) => ({ x: left + width - x, y: top + y }), true],
    [corners.bottomRight, ({ x, y }) => ({ x: left + width - x, y: top + height - y }), false],
    [corners.bottomLeft, ({ x, y }) => ({ x: left + x, y: top + height - y }), true],
  ]
  const commands: string[] = []
  for (const [corner, place, reversed] of placed) {
    const { points, radius } = grown(corner, offset)
    const ordered = (reversed ? [...points].reverse() : points).map(place)
    ordered.forEach((point, index) => {
      const to = `${number(point.x)} ${number(point.y)}`
      if (index > 0 && radius > 0) {
        commands.push(`A ${number(radius)} ${number(radius)} 0 0 1 ${to}`)
      } else {
        commands.push(`${commands.length === 0 ? 'M' : 'L'} ${to}`)
      }
    })
  }
  return `${commands.join(' ')} Z`
}

/** The CSS `clip-path` that cuts a `width` × `height` screen to its `corners`. */
export function screenClip(width: number, height: number, corners: ScreenCorners): string {
  return `path('${screenPath({ left: 0, top: 0, width, height }, corners)}')`
}

/** The silhouette around `screen`, concentric with its `corners`, with `buttons`. */
export function drawnFrame(screen: FrameBox, corners: ScreenCorners, buttons: readonly FrameButton[]): DrawnFrameShape {
  const bezel = drawnFrameBezel(screen)
  const body = {
    left: screen.left - bezel,
    top: screen.top - bezel,
    width: screen.width + 2 * bezel,
    height: screen.height + 2 * bezel,
  }
  const thickness = bezel * MARK
  const marks = buttons.map((button) => {
    const along = button.side === 'left' || button.side === 'right' ? body.height : body.width
    const start = button.start * along
    const length = Math.max(thickness, button.length * along)
    switch (button.side) {
      case 'left':
        return { side: button.side, left: body.left - thickness, top: body.top + start, width: thickness, height: length }
      case 'right':
        return { side: button.side, left: body.left + body.width, top: body.top + start, width: thickness, height: length }
      case 'top':
        return { side: button.side, left: body.left + start, top: body.top - thickness, width: length, height: thickness }
      case 'bottom':
        return { side: button.side, left: body.left + start, top: body.top + body.height, width: length, height: thickness }
    }
  })
  return {
    bezel,
    body: screenPath(screen, corners, bezel),
    band: screenPath(screen, corners, bezel * (1 - BAND / 2)),
    bandWidth: bezel * BAND,
    edge: screenPath(screen, corners, bezel - EDGE / 2),
    edgeWidth: EDGE,
    marks,
  }
}

/** How long an element's size must hold before it counts as laid out, so a drag asks once. */
const SETTLE_MS = 150

/**
 * Tells `settled` the element's size once a change of it has held for a
 * moment, not every pixel of a drag; answers what stops watching.
 */
export function observeSettledSize(element: Element, settled: (size: { width: number; height: number }) => void): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null
  const observer = new ResizeObserver(([entry]) => {
    if (!entry) {
      return
    }
    const size = { width: entry.contentRect.width, height: entry.contentRect.height }
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = setTimeout(() => {
      timer = null
      settled(size)
    }, SETTLE_MS)
  })
  observer.observe(element)
  return () => {
    observer.disconnect()
    if (timer !== null) {
      clearTimeout(timer)
    }
  }
}

import { expect, test } from 'bun:test'
import { DRAWN_FRAME_ROOM, SQUARE_CORNERS, drawnFrame, drawnFrameBezel, screenPath, type ScreenCorner, type ScreenCorners } from '../drawn-frame'

// The frame a view draws when the device's own is not installed (`mobile.md`
// § The live view): a body whose bezel is even all round, 4% of the
// screen's shorter side, concentric with the screen's corners whatever
// their shape, and each button a bar out of its side, where the fraction of
// the side puts it. Pure geometry, under a millisecond.

const screen = { left: 100, top: 50, width: 400, height: 800 }

function alike(corner: ScreenCorner): ScreenCorners {
  return { topLeft: corner, topRight: corner, bottomRight: corner, bottomLeft: corner }
}

/** The points a path passes through, arcs' ends included. */
function pathPoints(path: string): { x: number; y: number }[] {
  const points: { x: number; y: number }[] = []
  for (const [, command, numbers] of path.matchAll(/([MLA])([^MLAZ]*)/g)) {
    const values = numbers!.trim().split(/\s+/).map(Number)
    const [x, y] = command === 'A' ? values.slice(5) : values
    points.push({ x: x!, y: y! })
  }
  return points
}

/** The smallest distance from `point` to the polyline through `line`. */
function distance(point: { x: number; y: number }, line: { x: number; y: number }[]): number {
  let nearest = Infinity
  for (let index = 1; index < line.length; index += 1) {
    const [from, to] = [line[index - 1]!, line[index]!]
    const length = (to.x - from.x) ** 2 + (to.y - from.y) ** 2
    const along = length === 0 ? 0 : Math.max(0, Math.min(1, ((point.x - from.x) * (to.x - from.x) + (point.y - from.y) * (to.y - from.y)) / length))
    nearest = Math.min(nearest, Math.hypot(from.x + along * (to.x - from.x) - point.x, from.y + along * (to.y - from.y) - point.y))
  }
  return nearest
}

test('the bezel is 4% of the screen’s shorter side, and each button stands out of its side where its fractions put it', () => {
  const bezel = drawnFrameBezel(screen)
  expect(bezel).toBeCloseTo(0.04 * 400)
  const { body, marks } = drawnFrame(screen, SQUARE_CORNERS, [
    { side: 'left', start: 0.25, length: 0.1 },
    { side: 'right', start: 0.5, length: 0.1 },
    { side: 'top', start: 0.5, length: 0.25 },
    { side: 'bottom', start: 0, length: 0.1 },
  ])
  expect(pathPoints(body)[0]).toEqual({ x: screen.left - bezel, y: screen.top - bezel })
  const outer = { left: screen.left - bezel, top: screen.top - bezel, width: screen.width + 2 * bezel, height: screen.height + 2 * bezel }
  const [left, right, top, bottom] = marks
  // Each bar touches its side from outside, within the room the frame takes.
  expect(left!.left + left!.width).toBeCloseTo(outer.left)
  expect(screen.left - left!.left).toBeLessThanOrEqual(DRAWN_FRAME_ROOM * 400 + 1e-9)
  expect(right!.left).toBeCloseTo(outer.left + outer.width)
  expect(top!.top + top!.height).toBeCloseTo(outer.top)
  expect(bottom!.top).toBeCloseTo(outer.top + outer.height)
  // Along its side, where its fractions say.
  expect(left!.top).toBeCloseTo(outer.top + 0.25 * outer.height)
  expect(left!.height).toBeCloseTo(0.1 * outer.height)
  expect(right!.top).toBeCloseTo(outer.top + 0.5 * outer.height)
  expect(top!.left).toBeCloseTo(outer.left + 0.5 * outer.width)
  expect(top!.width).toBeCloseTo(0.25 * outer.width)
  expect(bottom!.left).toBeCloseTo(outer.left)
})

test('the body runs at an even distance from a curved corner, mirrored into each corner', () => {
  // A corner that reaches 60 pixels along each edge, flatter than a circle.
  const curve = Array.from({ length: 25 }, (_, index) => {
    const angle = (Math.PI / 2) * (index / 24)
    const power = 2 / 5
    return { x: 60 - 60 * Math.cos(angle) ** power, y: 60 - 60 * Math.sin(angle) ** power }
  })
  const corners = alike({ kind: 'curve', points: curve })
  const { body } = drawnFrame(screen, corners, [])
  const outer = pathPoints(body)
  // The screen's own edge: its four corners mirrored into place, as the clip draws it.
  const edge = pathPoints(screenPath(screen, corners))
  edge.push(edge[0]!)
  const gap = drawnFrameBezel(screen)
  for (const point of outer) {
    expect(distance(point, edge)).toBeCloseTo(gap, 0)
  }
  // The top right corner is the top left one mirrored: it leaves the right edge 60 pixels down.
  expect(edge).toContainEqual({ x: screen.left + screen.width, y: screen.top + 60 })
  expect(outer.some((point) => point.y < screen.top + 60 && point.x > screen.left + screen.width - 60)).toBe(true)
})

test('a round corner’s body is an arc a bezel wider, and a square one a sharp corner', () => {
  const bezel = drawnFrameBezel(screen)
  expect(drawnFrame(screen, alike({ kind: 'round', radius: 40 }), []).body).toContain(`A ${40 + bezel} ${40 + bezel} 0 0 1`)
  expect(drawnFrame(screen, SQUARE_CORNERS, []).body).not.toContain('A')
})

/**
 * Where a device's picture sits in its tab, and how the viewer's points on it become the
 * device's (`crates/plugin-mobile/page/src/index.ts`).
 *
 * The picture sits in the device's frame: the drawn one, laid into the area with the picture in
 * its screen, or the frame the view draws around the picture. Neither shows the screen larger
 * than its points, as Simulator's Point Accurate size shows it.
 */
import { DRAWN_FRAME_ROOM } from '@demicodes/plugin-sdk'
import type { LiveRect } from '../generated/plugin'

/** The margin around the device, in CSS pixels. */
export const MARGIN = 16

export interface Size {
  width: number
  height: number
}

/** A box in the area, in CSS pixels. */
export interface Placement {
  left: number
  top: number
  width: number
  height: number
}

/** The room a frame has in `area`: the area less its margin. */
export function frameRoom(area: Size): Size {
  return { width: Math.max(1, area.width - 2 * MARGIN), height: Math.max(1, area.height - 2 * MARGIN) }
}

/** `screen`, in points, fitted into `area` with room for the frame the view draws around it. */
export function placeScreen(screen: Size, area: Size): Placement {
  const room = frameRoom(area)
  // The frame grows with the screen: its room is a share of the shorter side.
  const around = 2 * DRAWN_FRAME_ROOM * Math.min(screen.width, screen.height)
  const scale = Math.min(1, room.width / (screen.width + around), room.height / (screen.height + around))
  const width = Math.round(screen.width * scale)
  const height = Math.round(screen.height * scale)
  return {
    left: Math.round((area.width - width) / 2),
    top: Math.round((area.height - height) / 2),
    width,
    height,
  }
}

/** A drawn frame of `frame` pixels whose screen is `screen` of them, for a screen of `points`. */
export interface DrawnFrame {
  width: number
  height: number
  screen: LiveRect
}

/** The drawn frame fitted into `area`, and the picture in its screen, in CSS pixels. */
export function placeInFrame(frame: DrawnFrame, points: Size, area: Size): { frame: Placement; screen: Placement } {
  const room = frameRoom(area)
  const scale = Math.min(room.width / frame.width, room.height / frame.height, points.width / frame.screen.width)
  const width = frame.width * scale
  const height = frame.height * scale
  const left = (area.width - width) / 2
  const top = (area.height - height) / 2
  return {
    frame: { left, top, width, height },
    screen: {
      left: left + frame.screen.x * scale,
      top: top + frame.screen.y * scale,
      width: frame.screen.width * scale,
      height: frame.screen.height * scale,
    },
  }
}

/** `frame` turned by `quarters` clockwise, as the device's frame of that orientation is drawn. */
export function turnedFrame(frame: DrawnFrame, quarters: number): DrawnFrame {
  let turned = frame
  for (let turn = 0; turn < ((quarters % 4) + 4) % 4; turn++) {
    const { width, height, screen } = turned
    turned = {
      width: height,
      height: width,
      screen: { x: height - screen.y - screen.height, y: screen.x, width: screen.height, height: screen.width },
    }
  }
  return turned
}

/** `box`, a box of `area`, turned by `quarters` clockwise about the area's center, as `rotate` turns it. */
export function turnedBox(box: Placement, area: Size, quarters: number): Placement {
  const turns = ((quarters % 4) + 4) % 4
  const across = { x: box.left + box.width / 2 - area.width / 2, y: box.top + box.height / 2 - area.height / 2 }
  const [x, y] = [[across.x, across.y], [-across.y, across.x], [-across.x, -across.y], [across.y, -across.x]][turns]!
  const [width, height] = turns % 2 === 0 ? [box.width, box.height] : [box.height, box.width]
  return { left: area.width / 2 + x! - width / 2, top: area.height / 2 + y! - height / 2, width, height }
}

/** The device's point under `inside`, a point of the area, clamped to the screen. */
export function devicePoint(inside: { x: number; y: number }, screen: Size, placed: Placement): { x: number; y: number } {
  const clamp = (value: number, max: number) => Math.max(0, Math.min(max, value))
  return {
    x: clamp(((inside.x - placed.left) / placed.width) * screen.width, screen.width),
    y: clamp(((inside.y - placed.top) / placed.height) * screen.height, screen.height),
  }
}

/** Where the device's point `point` shows in the area. */
export function areaPoint(point: { x: number; y: number }, screen: Size, placed: Placement): { x: number; y: number } {
  return {
    x: placed.left + (point.x / screen.width) * placed.width,
    y: placed.top + (point.y / screen.height) * placed.height,
  }
}

/**
 * Where a device being turned shows, relative to where its layout puts it:
 * turned `angle` degrees clockwise and scaled by `scale` about its screen's
 * center, then moved by `offset`.
 */
export interface TurnPose {
  angle: number
  scale: number
  offset: { x: number; y: number }
}

/** The device where its layout puts it. */
export const RESTING: TurnPose = { angle: 0, scale: 1, offset: { x: 0, y: 0 } }

function centerOf(placed: Placement): { x: number; y: number } {
  return { x: placed.left + placed.width / 2, y: placed.top + placed.height / 2 }
}

/**
 * The pose that shows a layout whose screen is `to` where `pose` showed one
 * whose screen was `from`, the device having turned `quarters` quarter turns
 * clockwise between them.
 */
export function poseBetween(from: Placement, pose: TurnPose, to: Placement, quarters: number): TurnPose {
  const [before, after] = [centerOf(from), centerOf(to)]
  // The old screen's width over the new one's side that turns back into it.
  const scale = from.width / (quarters % 2 === 0 ? to.width : to.height)
  return {
    angle: pose.angle - quarters * 90,
    scale: pose.scale * scale,
    offset: { x: before.x - after.x + pose.offset.x, y: before.y - after.y + pose.offset.y },
  }
}

/**
 * The pose a turn from `start` has reached at `progress`, 0 to 1, easing out
 * as a hand's turn does, scaled down where needed so that the device, of
 * `device` laid out, stays inside an area of `area` at every angle.
 */
export function poseAt(start: TurnPose, progress: number, device: Size, area: Size): TurnPose {
  const eased = 1 - (1 - Math.min(1, Math.max(0, progress))) ** 3
  const angle = start.angle * (1 - eased)
  const scale = start.scale + (1 - start.scale) * eased
  const radians = (angle * Math.PI) / 180
  const [cos, sin] = [Math.abs(Math.cos(radians)), Math.abs(Math.sin(radians))]
  const across = scale * (device.width * cos + device.height * sin)
  const down = scale * (device.width * sin + device.height * cos)
  const fit = Math.min(1, area.width / across, area.height / down)
  return { angle, scale: scale * fit, offset: { x: start.offset.x * (1 - eased), y: start.offset.y * (1 - eased) } }
}

/** The CSS transform of `pose` for a layout whose screen is `screen`, about the area's top left corner. */
export function poseTransform(pose: TurnPose, screen: Placement): string {
  const center = centerOf(screen)
  return `translate(${center.x + pose.offset.x}px, ${center.y + pose.offset.y}px) rotate(${pose.angle}deg) scale(${pose.scale}) translate(${-center.x}px, ${-center.y}px)`
}

/** The point of the layout whose screen is `screen` that `pose` shows at `at`, a point of the area. */
export function unposed(at: { x: number; y: number }, pose: TurnPose, screen: Placement): { x: number; y: number } {
  const center = centerOf(screen)
  const [x, y] = [(at.x - center.x - pose.offset.x) / pose.scale, (at.y - center.y - pose.offset.y) / pose.scale]
  const radians = (-pose.angle * Math.PI) / 180
  const [cos, sin] = [Math.cos(radians), Math.sin(radians)]
  return { x: center.x + x * cos - y * sin, y: center.y + x * sin + y * cos }
}

import type { InjectionKey, Ref } from 'vue'
import { clampSize, type ResizeBounds } from '../ui/resize-handle'

/**
 * The height of the windows the dock opens, such as Agents and Running: kept as
 * a share of the session area's height, so a window made tall in a large
 * browser window stays tall in a small one and never exceeds the room. The two
 * windows keep one share.
 */
export const DOCK_WINDOW_SHARE = { default: 0.5 } as const

/**
 * How many lines of the transcript the window keeps in view at its smallest.
 */
export const MIN_TRANSCRIPT_LINES = 3

/**
 * The parts of a window that stay at any height, measured from its layout, in
 * px.
 */
export interface WindowChrome {
  /** The strip of tabs. */
  header: number
  /**
   * A bar along the bottom edge, such as the status bar; 0 for a window with
   * none.
   */
  bar: number
  /** One line of the transcript. */
  line: number
}

/**
 * The px bounds a window's height moves within, in `room` px of space above the
 * dock.
 */
export function dockWindowBounds(chrome: WindowChrome, room: number): ResizeBounds {
  const max = Math.max(0, Math.round(room))
  return { min: Math.min(max, chrome.header + chrome.bar + MIN_TRANSCRIPT_LINES * chrome.line), max }
}

/** The window's height in px: its share of the area, held within the bounds. */
export function dockWindowHeight(share: number, area: number, bounds: ResizeBounds): number {
  return clampSize(share * area, bounds)
}

/**
 * The share a height in px is of the area; an unmeasured area has none to give,
 * so the default.
 */
export function dockWindowShare(height: number, area: number): number {
  return area > 0 ? height / area : DOCK_WINDOW_SHARE.default
}

/** What the area a window stands in tells it (`DockWindowArea`). */
export interface DockWindowSpace {
  /** The session area's height, which the share is of. */
  area: Ref<number>
  /** The height the window may take: the space above the dock. */
  room: Ref<number>
  /** The share the windows keep; a drag sets it as it moves. */
  share: Ref<number>
  /**
   * The share settled by a drag, a key or a double-click: the host keeps it.
   */
  commit(share: number): void
}

export const dockWindowKey: InjectionKey<DockWindowSpace> = Symbol('dock-window')

import { reactive, ref, type Ref } from 'vue'
import { z } from 'zod'
import { clamp } from '@demicodes/utils'
import { WINDOW_MIN, type WindowRect } from '../ui/floating-window'
import { moved, rearranged } from '../ui/drag-order'

/**
 * The windows of a conversation's work, and where each stands in the
 * conversation's area:
 *
 * ```text
 * +--------------------------------+-----------+
 * | conversation                   | docked 1  |
 * |        +-----------+           +-----------+
 * |        | floating  |           | docked 2  |
 * |        +-----------+           |           |
 * | [ composer                   ] |           |
 * +--------------------------------+-----------+
 * | + | pinned | open windows ...    | system  |   taskbar
 * +--------------------------------------------+
 * ```
 *
 * - Floating: where the user put it, over the conversation and the column. A
 *   window opens floating in the middle of the area, at a size to glance at
 *   what it shows.
 * - Anchored: in the area's bottom right corner, over the conversation and
 *   the composer's end, above the taskbar's system buttons that open it, at
 *   the size the user gave it; it does not move. A window that follows the
 *   conversation's own work, such as Agents, opens there, and goes from there
 *   to floating and back. One window stands there at a time: one that comes
 *   puts the one there to the taskbar, minimized.
 * - Docked: a row of the column at the area's right; the conversation narrows
 *   beside it. The column stacks rows only, never columns.
 * - Maximized: over the conversation's side of the area under its top bar,
 *   the composer included; the column stays beside it.
 * - Minimized: only its taskbar button shows.
 *
 * A window remembers where it stood before it was minimized, maximized or
 * closed, and its floating box, so each way back puts it there again. The
 * windows' order is the taskbar's: the order they came in, as the user
 * arranges it.
 *
 * The state is plain data, every box and size in shares of the area, so the
 * conversation keeps it and the same layout fits a screen of another size.
 */

export type Placement = 'floating' | 'anchored' | 'docked' | 'maximized' | 'minimized'

/** Where a window goes back to from minimized, maximized or closed. */
export type Return = 'floating' | 'anchored' | 'docked'

/** A size in shares of the area's width and height. */
export interface WindowSize {
  width: number
  height: number
}

/** A box in shares of the area's width and height, from its top left. */
export interface WindowBox {
  x: number
  y: number
  width: number
  height: number
}

export interface StageWindow {
  id: string
  placement: Placement
  return: Return
  /** The row it held in the column before it left it, which it takes again. */
  row: number
  /** Its box while floating. */
  box: WindowBox
  /** Its size while anchored; absent, it has its opening size there. */
  corner?: WindowSize
  /** Its place in the stacking order: the highest is in front. */
  order: number
}

/** What a closed window opens with again. */
export type ClosedWindow = Pick<StageWindow, 'return' | 'row' | 'box' | 'corner'>

/**
 * The most rows the column holds: three windows stacked still show each its
 * toolbar and a useful part of its content at a laptop's height.
 */
export const COLUMN_ROWS = 3

/** The least width of the column, in px: a file's lines and a page's controls stay readable. */
export const COLUMN_MIN_PX = 320

/** The least width the conversation keeps beside the column, in px: one message's line and the composer's controls. */
export const CONVERSATION_MIN_PX = 420

/** The column's share of the area's width at first. */
export const COLUMN_SHARE = 0.42

/**
 * The gap a docked or maximized window keeps from the area's edges and from
 * the next row, in px, so its edge and its corners show.
 */
export const WINDOW_GAP_PX = 8

/**
 * The gap between the conversation and the column, in px, wider than the
 * others: the grip that sizes the column stands in it with room either side.
 */
export const COLUMN_GUTTER_PX = 12

/**
 * How far into the area's width, from its right, a floating window dragged
 * there docks while the column is empty.
 */
export const DOCK_ZONE_SHARE = 0.25

/** How far a press moves before it drags a docked or maximized window out, in px. */
export const UNDOCK_PX = 6

/** A window's share of the area above the composer when it first opens. */
export const OPENING_SHARE = { width: 0.6, height: 0.72 }

/**
 * The largest box a window first opens in, in px: on a large screen a glance
 * reads a page or a file at its usual width, not across the whole screen.
 */
export const OPENING_MAX_PX = { width: 1100, height: 820 }

/**
 * The smallest box a window first opens in, in px, where the area has the
 * room: a file's bar and a page's toolbar fit in one row.
 */
export const OPENING_MIN_PX = { width: 480, height: 360 }

/** How far each window that opens where another floats moves down and to the right, in px. */
export const CASCADE_PX = 24

/**
 * An anchored window's share of the area until the user sizes it: a child's
 * transcript or a command's output reads there, and the conversation and the
 * composer's start still show beside it.
 */
export const CORNER_SHARE: WindowSize = { width: 0.45, height: 0.75 }

/** How long a window, or a taskbar button, takes to go to its new place or size, in ms. */
export const WINDOW_MOTION_MS = 160

export interface Size {
  width: number
  height: number
}

export interface Point {
  x: number
  y: number
}

/**
 * Where a dragged window lands on release, and the box that previews it:
 * docked into an empty column (`dock`), above or below a row, or in a row's
 * place, which a full column only allows (`full`).
 */
export type DropTarget =
  | { kind: 'dock' | 'above' | 'below'; row: number; rect: WindowRect }
  | { kind: 'replace'; row: number; rect: WindowRect; full: boolean }
  /** A window dragged out of the column, back into the row `row`; the row it left is `from`. */
  | { kind: 'move'; row: number; rect: WindowRect; from: number }

/** `box`'s px in an area of `size`. */
export function rectOfBox(box: WindowBox, size: Size): WindowRect {
  return { x: box.x * size.width, y: box.y * size.height, width: box.width * size.width, height: box.height * size.height }
}

/** `rect`'s shares of an area of `size`. */
export function boxOfRect(rect: WindowRect, size: Size): WindowBox {
  const width = Math.max(1, size.width)
  const height = Math.max(1, size.height)
  return { x: rect.x / width, y: rect.y / height, width: rect.width / width, height: rect.height / height }
}

/**
 * The box a window first opens in, in an area of `size` whose `bounds` lie
 * above the composer: centered in `bounds`, and moved down and right past each
 * of the `taken` px boxes that stands at the same place.
 */
export function openingBox(size: Size, bounds: Size, taken: readonly WindowRect[]): WindowBox {
  const width = Math.min(bounds.width, Math.max(OPENING_MIN_PX.width, Math.min(OPENING_MAX_PX.width, bounds.width * OPENING_SHARE.width)))
  const height = Math.min(bounds.height, Math.max(OPENING_MIN_PX.height, Math.min(OPENING_MAX_PX.height, bounds.height * OPENING_SHARE.height)))
  const middle = { x: (bounds.width - width) / 2, y: (bounds.height - height) / 2, width, height }
  return boxOfRect(cascade(middle, taken, bounds), size)
}

/**
 * Where a window that comes to `rect` stands, so that it never hides one of
 * `taken` whose corner stands within a step of its own: a step down and to
 * the right past each, as far as `bounds` lets it.
 */
export function cascade(rect: WindowRect, taken: readonly WindowRect[], bounds: Size): WindowRect {
  let { x, y } = rect
  const near = (other: WindowRect) => Math.abs(other.x - x) < CASCADE_PX && Math.abs(other.y - y) < CASCADE_PX
  while (taken.some(near) && y + rect.height + CASCADE_PX <= bounds.height && x + rect.width + CASCADE_PX <= bounds.width - WINDOW_GAP_PX) {
    x += CASCADE_PX
    y += CASCADE_PX
  }
  return { x: Math.round(x), y: Math.round(y), width: Math.round(rect.width), height: Math.round(rect.height) }
}

/**
 * A floating window's box for `free`, where a drag would put it, in an area of
 * `size`: inside the area, a gap from its edges. An edge that reaches the gap
 * stops there and the window narrows from the other side, which follows the
 * pointer, down to `WINDOW_MIN`, where the whole window stops. Computed from
 * the box the drag started with, so dragging back gives the size back; only
 * what a drag ends with is kept.
 */
export function keptInside(free: WindowRect, size: Size): WindowRect {
  const [x, width] = keptAlong(free.x, free.width, size.width, WINDOW_MIN.width)
  const [y, height] = keptAlong(free.y, free.height, size.height, WINDOW_MIN.height)
  return { x, y, width, height }
}

/** `keptInside` along one axis: the start and the length of a span inside `0..room`, at least `least` long. */
function keptAlong(start: number, length: number, room: number, least: number): [number, number] {
  const low = WINDOW_GAP_PX
  const high = Math.max(low, room - WINDOW_GAP_PX)
  const shortest = Math.min(least, high - low)
  // Whole pixels first: a pointer between two pixels, as on a Retina screen,
  // would otherwise round the near edge one way and the length the other,
  // and the far edge would jump a pixel back and forth as the window moves.
  const near = Math.round(start)
  let from = Math.max(near, low)
  let to = Math.min(near + Math.round(length), high)
  if (to - from < shortest) {
    if (near < low) {
      from = low
      to = low + shortest
    } else {
      to = high
      from = high - shortest
    }
  }
  return [Math.round(from), Math.round(to - from)]
}

/**
 * An anchored window's box in an area of `size`: its bottom right corner a gap
 * inside the area's, at `corner` or else `CORNER_SHARE`, kept inside it.
 */
export function cornerRect(corner: WindowSize | undefined, size: Size): WindowRect {
  const share = corner ?? CORNER_SHARE
  const width = Math.round(clamp(share.width * size.width, 0, Math.max(0, size.width - 2 * WINDOW_GAP_PX)))
  const height = Math.round(clamp(share.height * size.height, 0, Math.max(0, size.height - 2 * WINDOW_GAP_PX)))
  return { x: size.width - WINDOW_GAP_PX - width, y: size.height - WINDOW_GAP_PX - height, width, height }
}

/** The column's width for the area's `width` at `share`. */
export function columnWidth(width: number, share: number): number {
  const most = Math.max(COLUMN_MIN_PX, width - CONVERSATION_MIN_PX)
  return Math.round(clamp(share * width, Math.min(COLUMN_MIN_PX, width), most))
}

/** The column's box in an area of `size`. */
export function columnRect(size: Size, share: number): WindowRect {
  const width = columnWidth(size.width, share)
  return { x: size.width - width, y: 0, width, height: size.height }
}

/**
 * The boxes of `count` rows in the column `column`, each its share of the
 * height (`shares`, equal where absent), with the gaps between them and
 * around them.
 */
export function rowRects(column: WindowRect, count: number, shares: readonly number[] = []): WindowRect[] {
  if (count === 0) {
    return []
  }
  const inner = column.height - WINDOW_GAP_PX * (count + 1)
  const given = Array.from({ length: count }, (_, index) => shares[index] ?? 1 / count)
  const total = given.reduce((sum, share) => sum + share, 0)
  let y = column.y + WINDOW_GAP_PX
  return given.map((share) => {
    const height = Math.round((inner * share) / total)
    const rect = { x: column.x + COLUMN_GUTTER_PX, y, width: column.width - COLUMN_GUTTER_PX - WINDOW_GAP_PX, height }
    y += height + WINDOW_GAP_PX
    return rect
  })
}

/**
 * Where a window dragged to `point` lands, for a column holding `rows` other
 * windows; null where it stays floating.
 *
 * - An empty column takes a window dragged into the area's right quarter.
 * - Over a row, the row's top third puts the window above it, its middle third
 *   in its place, its bottom third below it.
 * - A full column only replaces.
 */
export function dropTarget(point: Point, size: Size, share: number, rows: number, rowShares: readonly number[] = []): DropTarget | null {
  if (point.y < 0 || point.y > size.height) {
    return null
  }
  const column = columnRect(size, share)
  if (rows === 0) {
    return point.x >= size.width * (1 - DOCK_ZONE_SHARE) ? { kind: 'dock', row: 0, rect: inset(column) } : null
  }
  if (point.x < column.x) {
    return null
  }
  const rects = rowRects(column, rows, rowShares)
  const found = rects.findIndex((rect) => point.y < rect.y + rect.height + WINDOW_GAP_PX)
  const index = found === -1 ? rects.length - 1 : found
  const rect = rects[index]!
  const third = (point.y - rect.y) / rect.height
  if (rows >= COLUMN_ROWS || (third > 1 / 3 && third < 2 / 3)) {
    return { kind: 'replace', row: index, rect, full: rows >= COLUMN_ROWS }
  }
  const at = third <= 1 / 3 ? index : index + 1
  return { kind: third <= 1 / 3 ? 'above' : 'below', row: at, rect: rowRects(column, rows + 1, joinedShares(rowShares, rows, at))[at]! }
}

/**
 * The rows' shares once a window joins the column's `rows` at `at`: it takes
 * an equal part, and the others keep the sizes the user gave them, among
 * themselves.
 */
function joinedShares(rowShares: readonly number[], rows: number, at: number): number[] {
  const others = Array.from({ length: rows }, (_, index) => rowShares[index] ?? 1 / Math.max(1, rows))
  const total = others.reduce((sum, share) => sum + share, 0)
  others.splice(at, 0, rows === 0 ? 1 : total / rows)
  const sum = others.reduce((all, share) => all + share, 0)
  return others.map((share) => share / sum)
}

/**
 * Where a window dragged out of the column lands while the pointer is over
 * the column: the row under the pointer, the one it left included, which
 * puts it back. The column keeps its `rows` while it moves, as only closing
 * a window or taking it out of the column takes a row away. The box is the
 * row as it stands once the others step aside for it, its own height moving
 * with it.
 */
export function columnSlot(point: Point, size: Size, share: number, rows: number, rowShares: readonly number[], from: number): DropTarget | null {
  const column = columnRect(size, share)
  if (point.x < column.x || point.y < 0 || point.y > size.height) {
    return null
  }
  const rects = rowRects(column, rows, rowShares)
  const found = rects.findIndex((rect) => point.y < rect.y + rect.height + WINDOW_GAP_PX)
  const row = found === -1 ? rects.length - 1 : found
  const after = rowShares.length === rows ? moved(rowShares, from, row) : []
  return { kind: 'move', row, rect: rowRects(column, rows, after)[row]!, from }
}

function inset(column: WindowRect): WindowRect {
  return { x: column.x + COLUMN_GUTTER_PX, y: column.y + WINDOW_GAP_PX, width: column.width - COLUMN_GUTTER_PX - WINDOW_GAP_PX, height: column.height - 2 * WINDOW_GAP_PX }
}

/** A conversation's windows, which it keeps. */
export interface StageState {
  windows: StageWindow[]
  /** The docked windows' ids, top to bottom. */
  column: string[]
  /** The column's share of the area's width. */
  share: number
  /** Each row's share of the column's height, top to bottom. */
  rowShares: number[]
  /** The window in front, which takes the keys. */
  focused: string | null
  /** The windows closed, by id, which open again where they stood. */
  closed: Record<string, ClosedWindow>
}

const boxSchema = z.object({ x: z.number(), y: z.number(), width: z.number(), height: z.number() })
const sizeSchema = z.object({ width: z.number(), height: z.number() })
const returnSchema = z.enum(['floating', 'anchored', 'docked'])
const stageStateSchema = z.object({
  windows: z.array(z.object({
    id: z.string(),
    placement: z.enum(['floating', 'anchored', 'docked', 'maximized', 'minimized']),
    return: returnSchema,
    row: z.number().int(),
    box: boxSchema,
    corner: sizeSchema.optional(),
    order: z.number(),
  })),
  column: z.array(z.string()).max(COLUMN_ROWS),
  share: z.number(),
  rowShares: z.array(z.number()),
  focused: z.string().nullable(),
  closed: z.record(z.string(), z.object({ return: returnSchema, row: z.number().int(), box: boxSchema, corner: sizeSchema.optional() })),
})

/** A layout the conversation kept, or null for one this page cannot read, as one an older page wrote. */
export function readStage(kept: unknown): StageState | null {
  const parsed = stageStateSchema.safeParse(kept)
  return parsed.success ? parsed.data : null
}

/**
 * The state of one area's windows, from `saved` when the conversation has one,
 * and the operations on it.
 *
 * `settled` counts the changes that ended: every operation, and a drag or a
 * resize once it is let go (`settle`), so the host keeps the state at each
 * and never while a window moves under the pointer.
 *
 * With `putAway`, every window of `saved` starts minimized, as a phone opens
 * a conversation on the conversation, not on a window over it.
 */
export function useWindowStage(saved?: StageState, { putAway = false }: { putAway?: boolean } = {}) {
  const state = reactive<StageState>(saved
    ? structuredClone(saved)
    : { windows: [], column: [], share: COLUMN_SHARE, rowShares: [], focused: null, closed: {} })
  let order = Math.max(0, ...state.windows.map((window) => window.order))
  const settled: Ref<number> = ref(0)

  /** `operation`, counted in `settled` once it ran. */
  function settling<A extends unknown[], R>(operation: (...args: A) => R): (...args: A) => R {
    return (...args) => {
      const result = operation(...args)
      settled.value += 1
      return result
    }
  }

  function find(id: string): StageWindow | undefined {
    return state.windows.find((window) => window.id === id)
  }

  function front(window: StageWindow): void {
    window.order = ++order
    state.focused = window.id
  }

  function leaveColumn(window: StageWindow): number {
    const row = state.column.indexOf(window.id)
    if (row >= 0) {
      state.column.splice(row, 1)
      state.rowShares.splice(row, 1)
    }
    return row
  }

  function newest(): string | null {
    const shown = state.windows.filter((window) => window.placement !== 'minimized').sort((a, b) => b.order - a.order)
    return shown[0]?.id ?? null
  }

  /**
   * Adds a window, minimized, with where it stood when it was closed, else
   * where it `opens`, its floating box `box`; `show` puts it there. One that
   * opens anchored always does, at the size it last had there: anchored is
   * where it belongs, and floating only a while.
   */
  function add(id: string, box: WindowBox, opens: Return = 'floating'): StageWindow {
    const existing = find(id)
    if (existing) {
      return existing
    }
    const closed = state.closed[id]
    const before = closed && opens !== 'anchored' ? closed : { return: opens, row: COLUMN_ROWS, box: closed?.box ?? box, corner: closed?.corner }
    delete state.closed[id]
    state.windows.push({ id, placement: 'minimized', ...before, order: 0 })
    return find(id)!
  }

  function remove(id: string): void {
    const window = find(id)
    if (!window) {
      return
    }
    const row = leaveColumn(window)
    state.closed[id] = {
      return: window.placement === 'docked' ? 'docked' : window.return,
      row: window.placement === 'docked' ? row : window.row,
      box: window.box,
      corner: window.corner,
    }
    state.windows.splice(state.windows.indexOf(window), 1)
    if (state.focused === id) {
      state.focused = newest()
    }
  }

  function dock(id: string, row: number): void {
    const window = find(id)
    if (!window) {
      return
    }
    leaveColumn(window)
    if (state.column.length >= COLUMN_ROWS) {
      float(id)
      return
    }
    const at = clamp(row, 0, state.column.length)
    state.rowShares = joinedShares(state.rowShares, state.column.length, at)
    state.column.splice(at, 0, id)
    window.placement = 'docked'
    window.return = 'docked'
    front(window)
  }

  /** Puts `id` back in the column as `column` orders it, its rows at `shares`: a docked window dragged along the column, or let go where it was. */
  function arrangeColumn(id: string, column: readonly string[], shares: readonly number[]): void {
    const window = find(id)
    if (!window) {
      return
    }
    leaveColumn(window)
    state.column = [...column]
    state.rowShares = [...shares]
    window.placement = 'docked'
    window.return = 'docked'
    front(window)
  }

  /** Puts `id` in the row `row` holds, which goes to the taskbar, minimized. */
  function replace(id: string, row: number): void {
    const window = find(id)
    const replaced = find(state.column[row] ?? '')
    if (!window || !replaced || replaced === window) {
      return
    }
    leaveColumn(window)
    const at = state.column.indexOf(replaced.id)
    state.column.splice(at, 1, id)
    replaced.placement = 'minimized'
    replaced.return = 'docked'
    replaced.row = at
    window.placement = 'docked'
    window.return = 'docked'
    front(window)
  }

  function float(id: string): void {
    const window = find(id)
    if (!window) {
      return
    }
    leaveColumn(window)
    window.placement = 'floating'
    window.return = 'floating'
    front(window)
  }

  function anchor(id: string): void {
    const window = find(id)
    if (!window) {
      return
    }
    for (const other of state.windows) {
      if (other !== window && other.placement === 'anchored') {
        other.placement = 'minimized'
      }
    }
    leaveColumn(window)
    window.placement = 'anchored'
    window.return = 'anchored'
    front(window)
  }

  function maximize(id: string): void {
    const window = find(id)
    if (!window) {
      return
    }
    for (const other of state.windows) {
      if (other !== window && other.placement === 'maximized') {
        restore(other.id)
      }
    }
    if (window.placement === 'docked') {
      window.row = leaveColumn(window)
    }
    window.placement = 'maximized'
    front(window)
  }

  function minimize(id: string): void {
    const window = find(id)
    if (!window || window.placement === 'minimized') {
      return
    }
    if (window.placement === 'docked') {
      window.row = leaveColumn(window)
    }
    window.placement = 'minimized'
    if (state.focused === id) {
      state.focused = newest()
    }
  }

  /** Puts `id` back where it returns to: its row while the column has room, else floating; or its corner. */
  function restore(id: string): void {
    const window = find(id)
    if (!window) {
      return
    }
    if (window.return === 'docked' && state.column.length < COLUMN_ROWS) {
      dock(id, window.row)
    } else if (window.return === 'anchored') {
      anchor(id)
    } else {
      float(id)
    }
  }

  /** Shows `id`: in front where it stands, else where it returns to. */
  function show(id: string): void {
    const window = find(id)
    if (!window) {
      return
    }
    if (window.placement === 'minimized') {
      restore(id)
    } else {
      front(window)
    }
  }

  /** What a press on its taskbar button does: shows it, or minimizes it while it is in front. */
  function toggle(id: string): void {
    const window = find(id)
    if (!window) {
      return
    }
    if (window.placement !== 'minimized' && state.focused === id) {
      minimize(id)
    } else {
      show(id)
    }
  }

  /** Puts the windows `ids` name in that order among the slots they hold, as the taskbar lists them. */
  function arrange(ids: readonly string[]): void {
    const order = ids.flatMap((id) => find(id) ?? [])
    state.windows = rearranged(state.windows, order)
  }

  function focus(id: string): void {
    const window = find(id)
    if (window && state.focused !== id) {
      front(window)
    }
  }

  /**
   * Minimizes every window, as a phone's Back returns to the conversation
   * from whichever window it shows, the others standing unseen behind it.
   */
  function minimizeAll(): void {
    for (const window of state.windows) {
      minimize(window.id)
    }
  }

  if (putAway) {
    minimizeAll()
  }

  return {
    state,
    settled,
    find,
    add: settling(add),
    remove: settling(remove),
    dock: settling(dock),
    replace: settling(replace),
    arrangeColumn: settling(arrangeColumn),
    float: settling(float),
    /**
     * Floats `id` as a drag takes it out of the column or out of maximized:
     * kept only once the drag is let go, by `settle` or the operation it ends
     * with, never while it goes on.
     */
    lift: float,
    anchor: settling(anchor),
    maximize: settling(maximize),
    minimize: settling(minimize),
    minimizeAll: settling(minimizeAll),
    restore: settling(restore),
    show: settling(show),
    toggle: settling(toggle),
    arrange: settling(arrange),
    focus: settling(focus),
    /** A drag or a resize of a window, or of the column or its rows, was let go. */
    settle: () => {
      settled.value += 1
    },
  }
}

export type WindowStageModel = ReturnType<typeof useWindowStage>

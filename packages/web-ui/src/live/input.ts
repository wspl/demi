/**
 * The viewer's input as the live view carries it (`mobile.md` § The live view): the events of the user's own browser, in
 * the picture's coordinates, whatever the source turns them into.
 */
import type { ViewerInput } from '@demicodes/protocol'

/** Modifier keys as the protocol numbers them. */
export const ALT = 1
export const CONTROL = 2
export const META = 4
export const SHIFT = 8

export interface ModifierState {
  altKey: boolean
  ctrlKey: boolean
  metaKey: boolean
  shiftKey: boolean
}

export function modifiers(event: ModifierState): number {
  return (event.altKey ? ALT : 0) | (event.ctrlKey ? CONTROL : 0)
    | (event.metaKey ? META : 0) | (event.shiftKey ? SHIFT : 0)
}

type PointerButton = 'none' | 'left' | 'middle' | 'right'
const BUTTONS: readonly PointerButton[] = ['left', 'middle', 'right']

export interface PointerInput extends ModifierState {
  button: number
  buttons: number
}

/** The longest pause between two presses of one double or triple click. */
const CLICK_INTERVAL_MS = 500
/** The farthest the pointer moves between two presses of one double or triple click, in CSS pixels. */
const CLICK_DISTANCE = 4

/**
 * Counts the viewer's consecutive presses as the source's click count, which
 * a pointer event does not carry: its `detail` is 0, so without this a
 * double click reached a page as two single clicks and selected no word.
 */
export class ClickCount {
  private last: { time: number; x: number; y: number; button: number; count: number } | null = null

  /** The click count of a press at `event`: one more than the press before when it follows it closely. */
  press(event: { timeStamp: number; clientX: number; clientY: number; button: number }): number {
    const last = this.last
    const follows = last !== null
      && last.button === event.button
      && event.timeStamp - last.time <= CLICK_INTERVAL_MS
      && Math.hypot(event.clientX - last.x, event.clientY - last.y) <= CLICK_DISTANCE
    const count = follows ? Math.min(3, last.count + 1) : 1
    this.last = { time: event.timeStamp, x: event.clientX, y: event.clientY, button: event.button, count }
    return count
  }

  /** The count of the last press, which its release carries. */
  get current(): number {
    return this.last?.count ?? 1
  }
}

function pressedButton(event: PointerInput, action: 'move' | 'down' | 'up'): PointerButton {
  if (action !== 'move') {
    return BUTTONS[event.button] ?? 'none'
  }
  if (event.buttons & 1) {
    return 'left'
  }
  if (event.buttons & 2) {
    return 'right'
  }
  return event.buttons & 4 ? 'middle' : 'none'
}

/** A mouse's pointer event at the picture's `point`, a press or release the `clicks`th of its click. */
export function pointerInput(
  action: 'move' | 'down' | 'up',
  point: { x: number; y: number },
  event: PointerInput,
  clicks = 1,
): ViewerInput {
  return {
    type: 'pointer',
    action,
    x: point.x,
    y: point.y,
    button: pressedButton(event, action),
    buttons: event.buttons,
    clickCount: action === 'move' ? 0 : clicks,
    modifiers: modifiers(event),
  }
}

/** The most fingers a touch carries, as the protocol bounds it. */
const MAX_CONTACTS = 10

/**
 * Fingers on the picture, for a source that takes touches: those down after
 * a start or a move, the one lifted with an end, none with a cancel.
 */
export function touchInput(
  action: 'start' | 'move' | 'end' | 'cancel',
  contacts: readonly { id: number; x: number; y: number }[],
): ViewerInput {
  return {
    type: 'touch',
    action,
    contacts: action === 'cancel' ? [] : contacts.slice(0, MAX_CONTACTS).map(({ id, x, y }) => ({ id, x, y })),
  }
}

export interface WheelInput extends ModifierState {
  deltaX: number
  deltaY: number
  /** 0 pixels, 1 lines, 2 pages, as the DOM numbers them. */
  deltaMode: number
}

/** A wheel turn in the picture's pixels, whatever units the viewer's web browser used. */
export function wheelInput(
  point: { x: number; y: number },
  event: WheelInput,
  viewportHeight: number,
): ViewerInput {
  const lines = 16
  const scale = event.deltaMode === 1 ? lines : event.deltaMode === 2 ? viewportHeight : 1
  const delta = (value: number) => Math.max(-10_000, Math.min(10_000, value * scale))
  return {
    type: 'wheel',
    x: point.x,
    y: point.y,
    deltaX: delta(event.deltaX),
    deltaY: delta(event.deltaY),
    modifiers: modifiers(event),
  }
}

export interface KeyInput extends ModifierState {
  key: string
  code: string
  keyCode: number
  repeat: boolean
  location: number
  getModifierState(key: string): boolean
}

/** A key the source receives as it is, with the character it types. */
export function keyInput(action: 'down' | 'up', event: KeyInput): ViewerInput {
  const altGraph = event.getModifierState('AltGraph')
  const shortcut = (event.ctrlKey || event.metaKey) && !altGraph
  // One event carries its character, so a page gets keypress and can cancel it.
  const text = action === 'down' && [...event.key].length === 1 && !shortcut ? event.key : undefined
  return {
    type: 'key',
    action,
    key: event.key.slice(0, 64),
    code: event.code.slice(0, 64),
    keyCode: Math.max(0, Math.min(255, event.keyCode)),
    modifiers: modifiers(event),
    repeat: action === 'down' && event.repeat,
    location: Math.max(0, Math.min(3, event.location)),
    ...(text === undefined ? {} : { text }),
    altGraph,
  }
}

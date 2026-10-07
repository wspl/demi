/**
 * The viewer's input as the live protocol carries it
 * (`live-view.md` § Input): the events of the user's own browser,
 * in the watched tab's CSS coordinates.
 */
import type { LiveViewerMessage } from '../generated/plugin'

/** Modifier keys as CDP numbers them. */
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
 * Counts the viewer's consecutive presses as the page's click count, which
 * a pointer event does not carry: its `detail` is 0, so without this a
 * double click reached the page as two single clicks and selected no word.
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

/** A pointer event in the tab, at the tab's coordinates, a press or release the `clicks`th of its click. */
export function pointerMessage(
  tab: string,
  action: 'move' | 'down' | 'up',
  point: { x: number; y: number },
  event: PointerInput,
  clicks = 1,
): LiveViewerMessage {
  return {
    type: 'pointer',
    tab,
    action,
    x: point.x,
    y: point.y,
    button: pressedButton(event, action),
    buttons: event.buttons,
    clickCount: action === 'move' ? 0 : clicks,
    modifiers: modifiers(event),
  }
}

export interface WheelInput extends ModifierState {
  deltaX: number
  deltaY: number
  /** 0 pixels, 1 lines, 2 pages, as the DOM numbers them. */
  deltaMode: number
}

/** A wheel turn in the tab's CSS pixels, whatever units the viewer's web browser used. */
export function wheelMessage(
  tab: string,
  point: { x: number; y: number },
  event: WheelInput,
  viewportHeight: number,
): LiveViewerMessage {
  const lines = 16
  const scale = event.deltaMode === 1 ? lines : event.deltaMode === 2 ? viewportHeight : 1
  const delta = (value: number) => Math.max(-10_000, Math.min(10_000, value * scale))
  return {
    type: 'wheel',
    tab,
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

/** A key the page sends as it is, with the character it types. */
export function keyMessage(tab: string, action: 'down' | 'up', event: KeyInput): LiveViewerMessage {
  const altGraph = event.getModifierState('AltGraph')
  const shortcut = (event.ctrlKey || event.metaKey) && !altGraph
  // One event carries its character, so the page gets keypress and can cancel it.
  const text = action === 'down' && [...event.key].length === 1 && !shortcut ? event.key : undefined
  return {
    type: 'key',
    tab,
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

/** Keys the viewer's own web browser keeps: its paste reaches the page as a paste. */
export function localKey(event: KeyInput): boolean {
  const shortcut = (event.ctrlKey || event.metaKey) && !event.getModifierState('AltGraph')
  return shortcut && event.key.toLowerCase() === 'v'
}

/** What a browser's own shortcut does in the panel, rather than in the page. */
export type BrowserShortcut = 'address' | 'back' | 'forward' | 'reload'

/**
 * The browser's shortcut a key is, with the focus in the panel
 * (`live-view.md` § A browser tab in the panel): on a Mac ⌘L focuses the
 * address bar, ⌘[ and ⌘] go Back and Forward and ⌘R reloads; elsewhere
 * Control+L, Alt+Left and Alt+Right, and Control+R or F5, as Chrome has them.
 * Null for a key the page gets.
 */
export function browserShortcut(event: KeyInput, platform: 'mac' | 'windows' | 'linux' | 'other'): BrowserShortcut | null {
  const command = platform === 'mac'
    ? event.metaKey && !event.ctrlKey && !event.altKey
    : event.ctrlKey && !event.metaKey && !event.altKey
  const alone = !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
  if (command && !event.shiftKey) {
    switch (event.code) {
      case 'KeyL':
        return 'address'
      case 'KeyR':
        return 'reload'
      case 'BracketLeft':
        return platform === 'mac' ? 'back' : null
      case 'BracketRight':
        return platform === 'mac' ? 'forward' : null
    }
  }
  if (platform !== 'mac' && event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey) {
    if (event.code === 'ArrowLeft') {
      return 'back'
    }
    if (event.code === 'ArrowRight') {
      return 'forward'
    }
  }
  return alone && event.code === 'F5' ? 'reload' : null
}

/**
 * The key presses of the platform's editing shortcut for `letter`, such as
 * ⌘C on a Mac and Control+C elsewhere, as the viewer's own keys would send
 * them: the browser's menu copies, cuts and pastes in the page this way.
 */
export function shortcutKeys(tab: string, letter: 'c' | 'x', platform: 'mac' | 'windows' | 'linux' | 'other'): LiveViewerMessage[] {
  const pressed: KeyInput = {
    key: letter,
    code: `Key${letter.toUpperCase()}`,
    keyCode: letter.toUpperCase().charCodeAt(0),
    repeat: false,
    location: 0,
    altKey: false,
    shiftKey: false,
    ctrlKey: platform !== 'mac',
    metaKey: platform === 'mac',
    getModifierState: () => false,
  }
  return [keyMessage(tab, 'down', pressed), keyMessage(tab, 'up', pressed)]
}

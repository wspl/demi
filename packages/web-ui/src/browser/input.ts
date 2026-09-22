/**
 * The viewer's input as the live protocol carries it
 * (`browser-live-view.md` § Input): the events of the user's own browser,
 * in the watched tab's CSS coordinates.
 */
import type { LiveViewerMessage } from '@demicodes/browser-protocol/live'

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

/** The platform whose shortcuts the viewer's keys carry. */
export function viewerPlatform(agent: { platform?: string; userAgent: string }): 'mac' | 'windows' | 'linux' | 'other' {
  const name = `${agent.platform ?? ''} ${agent.userAgent}`
  if (/mac/i.test(name)) {
    return 'mac'
  }
  if (/win/i.test(name)) {
    return 'windows'
  }
  return /linux|android|cros/i.test(name) ? 'linux' : 'other'
}

type PointerButton = 'none' | 'left' | 'middle' | 'right'
const BUTTONS: readonly PointerButton[] = ['left', 'middle', 'right']

export interface PointerInput extends ModifierState {
  button: number
  buttons: number
  detail: number
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

/** A pointer event in the tab, at the tab's coordinates. */
export function pointerMessage(
  tab: string,
  action: 'move' | 'down' | 'up',
  point: { x: number; y: number },
  event: PointerInput,
): LiveViewerMessage {
  return {
    type: 'pointer',
    tab,
    action,
    x: point.x,
    y: point.y,
    button: pressedButton(event, action),
    buttons: event.buttons,
    clickCount: action === 'move' ? 0 : Math.min(3, event.detail || 1),
    modifiers: modifiers(event),
  }
}

export interface WheelInput extends ModifierState {
  deltaX: number
  deltaY: number
  /** 0 pixels, 1 lines, 2 pages, as the DOM numbers them. */
  deltaMode: number
}

/** A wheel turn in the tab's CSS pixels, whatever units the viewer's browser used. */
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

/** Keys the viewer's own browser keeps: its paste reaches the page as a paste. */
export function localKey(event: KeyInput): boolean {
  const shortcut = (event.ctrlKey || event.metaKey) && !event.getModifierState('AltGraph')
  return shortcut && event.key.toLowerCase() === 'v'
}

/** A key that arrives while an input method is composing belongs to the composition. */
export function composingKey(event: { keyCode: number; key: string; isComposing?: boolean }): boolean {
  return Boolean(event.isComposing) || event.keyCode === 229 || event.key === 'Dead'
}

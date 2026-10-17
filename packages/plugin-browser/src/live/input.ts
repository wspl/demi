/**
 * The viewer's input to a browser tab (`live-view.md` § Input): the live
 * view's input (the SDK's `pointerInput`, `wheelInput`, `keyInput`) to the
 * watched tab, and the keys the browser's panel keeps for itself.
 */
import { keyInput, type KeyInput, type ViewerInput } from '@demicodes/plugin-sdk'
import type { LiveViewerMessage } from '../generated/plugin'

/** The viewer's `input` to the browser tab `tab`. */
export function inputMessage(tab: string, input: ViewerInput): LiveViewerMessage {
  return { type: 'input', tab, input }
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
  return [inputMessage(tab, keyInput('down', pressed)), inputMessage(tab, keyInput('up', pressed))]
}

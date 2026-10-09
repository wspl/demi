// `demi.ime(text, { into, commit })`: composes text through an input method
// as a Chinese or Japanese input method does: the field sees a composition
// grow character by character, then the key that ends it: `commit: 'enter'`
// (the default) commits the candidate, `'escape'` cancels the composition,
// and `'none'` leaves it open. `into` (a Playwright locator) is focused
// first. Each engine gets its own order of events, the one its browser
// produces on macOS:
//
//   Chromium                          WebKit (Safari)
//   keydown Enter  229 isComposing    compositionend
//   compositionend                    keydown Enter  229, isComposing false
//   keyup Enter    13                 keyup Enter    13
//
// Playwright types and inserts text but composes none. In Chromium this uses
// the DevTools protocol's input method events, so the browser itself
// composes; WebKit's protocol has none, so on a page of `demi.webkit()` the
// events are dispatched in Safari's order and the text is inserted while
// the composition is open, which is what a page sees of Safari's input
// method, though its events are not trusted ones.
//
// The ending key's keydown names no native key code. On macOS the protocol
// takes `nativeVirtualKeyCode` as the Mac key code of the NSEvent the
// browser builds for the key, and a keydown the page lets through
// unhandled, as a field keeps a composing key without preventing its
// default, goes back to the browser's own menus as that NSEvent. 229 is no
// Mac key: given as one, the browser matched it to its own commands (a
// plain page's tab went to chrome://settings/help, Chrome's About), and
// with a menu that opens and closes in the same call its process spun in
// AppKit's key equivalents and answered nothing more. Playwright's own key
// presses give no native code either.
import type { Locator } from 'playwright'
import type { Tool } from '../tool'

export interface ImeOptions {
  into?: Locator
  commit?: 'enter' | 'escape' | 'none'
}

/** The key that ends a composition, as the page's events name it. */
const ENDING = {
  enter: { key: 'Enter', code: 'Enter', keyCode: 13 },
  escape: { key: 'Escape', code: 'Escape', keyCode: 27 },
} as const

export async function ime(tool: Tool, text: string, options: ImeOptions = {}): Promise<void> {
  const commit = options.commit ?? 'enter'
  if (options.into !== undefined) {
    await options.into.focus()
    if (options.into.page().context().browser()?.browserType().name() === 'webkit') {
      await options.into.page().evaluate(safariComposition, { text, commit })
      return
    }
  }
  const cdp = await tool.browser.cdp()
  const characters = [...text]
  for (let count = 1; count <= characters.length; count += 1) {
    const composing = characters.slice(0, count).join('')
    await cdp.send('Input.imeSetComposition', {
      text: composing,
      selectionStart: composing.length,
      selectionEnd: composing.length,
    })
  }
  if (commit === 'none') {
    return
  }
  // The key's keydown reaches the page as 229 while the input method holds
  // it, the input method commits or cancels the composition, and the keyup
  // is a plain key's.
  const key = ENDING[commit]
  await cdp.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown',
    key: key.key,
    code: key.code,
    windowsVirtualKeyCode: 229,
  })
  if (commit === 'enter') {
    await cdp.send('Input.insertText', { text })
  } else {
    await cdp.send('Input.imeSetComposition', { text: '', selectionStart: 0, selectionEnd: 0 })
  }
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: key.key, code: key.code, windowsVirtualKeyCode: key.keyCode })
}

/**
 * Runs in the page: Safari's events for a composition of `text` in the
 * focused field, ended by `commit`. Safari ends the composition before the
 * ending key's keydown, which it marks only with key code 229.
 */
function safariComposition({ text, commit }: { text: string, commit: 'enter' | 'escape' | 'none' }): void {
  const field = document.activeElement
  if (field === null) {
    throw new Error('Nothing has the focus to compose in')
  }
  const composition = (type: string, data: string) =>
    field.dispatchEvent(new CompositionEvent(type, { bubbles: true, cancelable: true, data }))
  const key = (type: string, name: string, keyCode: number) =>
    field.dispatchEvent(new KeyboardEvent(type, {
      bubbles: true,
      cancelable: true,
      composed: true,
      key: name,
      code: name,
      keyCode,
      which: keyCode,
    }))
  composition('compositionstart', '')
  const characters = [...text]
  for (let count = 1; count <= characters.length; count += 1) {
    composition('compositionupdate', characters.slice(0, count).join(''))
  }
  // The composed text stands in the field while the composition is open.
  document.execCommand('insertText', false, text)
  if (commit === 'none') {
    return
  }
  if (commit === 'escape') {
    for (let count = 0; count < characters.length; count += 1) {
      document.execCommand('delete')
    }
  }
  composition('compositionend', commit === 'enter' ? text : '')
  const name = commit === 'enter' ? 'Enter' : 'Escape'
  key('keydown', name, 229)
  key('keyup', name, commit === 'enter' ? 13 : 27)
}

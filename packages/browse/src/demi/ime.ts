// `demi.ime(text, { into, commit })`: composes text through an input method
// and commits it, as a Chinese or Japanese input method does: the field sees
// a composition grow character by character, then the Enter that commits
// the candidate, whose keydown carries keyCode 229 and must not submit.
// Playwright types and inserts text but composes none, so this uses the
// DevTools protocol's input method events. `into` (a Playwright locator) is
// focused first; `commit: 'none'` leaves the composition open.
import type { Locator } from 'playwright'
import type { Tool } from '../tool'

export interface ImeOptions {
  into?: Locator
  commit?: 'enter' | 'none'
}

export async function ime(tool: Tool, text: string, options: ImeOptions = {}): Promise<void> {
  if (options.into !== undefined) {
    await options.into.focus()
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
  if (options.commit === 'none') {
    return
  }
  // Chrome's order on macOS: the Enter's keydown reaches the page as 229
  // while the input method holds it, the candidate is inserted, and the
  // keyup is a plain Enter's.
  await cdp.send('Input.dispatchKeyEvent', {
    type: 'rawKeyDown',
    key: 'Enter',
    code: 'Enter',
    windowsVirtualKeyCode: 229,
    nativeVirtualKeyCode: 229,
  })
  await cdp.send('Input.insertText', { text })
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 })
}

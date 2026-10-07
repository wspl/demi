// `bun check ime <text>`: composes text through an input method and commits
// it, as a Chinese or Japanese input method does: the field sees a
// composition grow character by character, then the Enter that commits the
// candidate, whose keydown carries keyCode 229 and must not submit.
// Playwright types and inserts text but composes none, so this uses the
// DevTools protocol's input method events.
import { CheckFailure, parse, type Context } from '../command'
import { element } from '../find'

const USAGE = 'ime <text> [--into <locator>] [--commit enter|none]'
const OPTIONS = { into: { type: 'string' }, commit: { type: 'string' } } as const

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  const [text] = positionals
  const commit = values.commit ?? 'enter'
  if (text === undefined || positionals.length !== 1 || (commit !== 'enter' && commit !== 'none')) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  if (values.into !== undefined) {
    await (await element(context, values.into)).focus()
  }
  const cdp = await context.browser.cdp()
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
    context.print(`Composing ${text}, not committed`)
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
  context.print(`Composed ${text} and committed it with Enter`)
}

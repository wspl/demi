// `bun browse focus`: prints the element that has the keyboard focus, as a
// person would name it: its role or tag, its accessible label or text, and
// whether it takes typing, so a check that the next key lands in the right
// place needs no script. `--expect <locator>` fails unless that element is
// the focused one.
import { CommandFailure, parse, type Context } from '../command'
import { element } from '../find'

const USAGE = 'focus [--expect <locator>]'
const OPTIONS = { expect: { type: 'string' } } as const

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  if (positionals.length > 0) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const page = await context.browser.page()
  const focused = await page.evaluate(() => {
    const active = document.activeElement
    if (!active || active === document.body) {
      return null
    }
    const label = active.getAttribute('aria-label') ?? active.getAttribute('title') ?? active.textContent?.trim().slice(0, 60) ?? ''
    const editable = active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement
      || (active instanceof HTMLElement && active.isContentEditable)
    return { kind: active.getAttribute('role') ?? active.tagName.toLowerCase(), label, editable }
  })
  context.print(focused === null
    ? 'Nothing has the focus (the page body)'
    : `${focused.kind} "${focused.label}"${focused.editable ? ', takes typing' : ''}`)
  if (values.expect !== undefined) {
    const expected = await element(context, values.expect)
    const isFocused = await expected.evaluate((node) => node === document.activeElement || node.contains(document.activeElement))
    if (!isFocused) {
      throw new CommandFailure(`${values.expect} does not have the focus`)
    }
  }
}

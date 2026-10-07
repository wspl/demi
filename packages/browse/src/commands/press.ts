// `bun browse press <key>...`: presses keys one after another, each a key or
// a shortcut in Playwright's names: Enter, Escape, ArrowDown, Meta+Comma.
import { CommandFailure, parse, type Context } from '../command'
import { reachable } from '../find'

const USAGE = 'press <key>... [--into <locator>]'
const OPTIONS = { into: { type: 'string' } } as const

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  if (positionals.length === 0) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  if (values.into !== undefined) {
    const target = await reachable(context, values.into, 'keys')
    for (const key of positionals) {
      await target.press(key)
    }
    return
  }
  const page = await context.browser.page()
  for (const key of positionals) {
    await page.keyboard.press(key)
  }
}

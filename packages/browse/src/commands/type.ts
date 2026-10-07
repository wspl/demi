// `bun browse type <text>`: types text key by key, into the focused element
// or into the one `--into` names, so every key's events fire.
import { CommandFailure, numeric, parse, type Context } from '../command'
import { element } from '../find'

const USAGE = 'type <text> [--into <locator>] [--delay <ms>]'
const OPTIONS = { into: { type: 'string' }, delay: { type: 'string' } } as const

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  if (positionals.length !== 1) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const delay = values.delay === undefined ? 0 : numeric(values.delay, '--delay')
  if (values.into !== undefined) {
    await (await element(context, values.into)).pressSequentially(positionals[0], { delay })
    return
  }
  const page = await context.browser.page()
  await page.keyboard.type(positionals[0], { delay })
}

// `bun check fill <locator> <text>`: replaces the text of a field, a text
// area or an editable element at once, as pasting does.
import { CheckFailure, parse, type Context } from '../command'
import { element } from '../find'

const USAGE = 'fill <locator> <text>'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  if (positionals.length !== 2) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const field = await element(context, positionals[0])
  await field.fill(positionals[1], { timeout: 10_000 })
}

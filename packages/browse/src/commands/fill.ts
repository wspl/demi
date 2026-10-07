// `bun browse fill <locator> <text>`: replaces the text of a field, a text
// area or an editable element at once, as pasting does.
import { CommandFailure, parse, type Context } from '../command'
import { reachable } from '../find'

const USAGE = 'fill <locator> <text>'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  if (positionals.length !== 2) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const field = await reachable(context, positionals[0], 'typing')
  await field.fill(positionals[1], { timeout: 10_000 })
}

// `bun browse select <locator> <option>...`: chooses options of a <select>,
// each named by its value or its label.
import { CommandFailure, parse, type Context } from '../command'
import { reachable } from '../find'

const USAGE = 'select <locator> <option>...'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const [locator, ...options] = positionals
  if (locator === undefined || options.length === 0) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const chosen = await (await reachable(context, locator, 'click')).selectOption(options, { timeout: 10_000 })
  context.print(`Selected ${chosen.join(', ')}`)
}

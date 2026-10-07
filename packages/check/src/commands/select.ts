// `bun check select <locator> <option>...`: chooses options of a <select>,
// each named by its value or its label.
import { CheckFailure, parse, type Context } from '../command'
import { element } from '../find'

const USAGE = 'select <locator> <option>...'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const [locator, ...options] = positionals
  if (locator === undefined || options.length === 0) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const chosen = await (await element(context, locator)).selectOption(options, { timeout: 10_000 })
  context.print(`Selected ${chosen.join(', ')}`)
}

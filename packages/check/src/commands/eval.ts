// `bun check eval <javascript>`: evaluates an expression in the page, awaits
// it when it is a promise, and prints the result as JSON.
import { CheckFailure, parse, type Context } from '../command'

const USAGE = 'eval <javascript>'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  if (positionals.length !== 1) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const page = await context.browser.page()
  const result: unknown = await page.evaluate(positionals[0])
  context.print(result === undefined ? 'undefined' : JSON.stringify(result, null, 2))
}

// `bun browse eval <javascript>`: evaluates an expression in the page, awaits
// it when it is a promise, and prints the result as JSON. The expression may
// await, as `(await fetch('/api/sync')).status`; a function, such as
// `async () => …`, is called and its result printed.
import { CommandFailure, parse, type Context } from '../command'

const USAGE = 'eval <javascript>'

/** An `await` that is a word of the expression, not a part of another name or a property. */
const AWAIT = /(?:^|[^\w$.])await\b/
/** An expression that is a function, which Playwright calls itself. */
const FUNCTION = /^\s*(?:async\b|function\b|\([^)]*\)\s*=>|[\w$]+\s*=>)/

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  if (positionals.length !== 1) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const [expression] = positionals
  // Playwright evaluates a string as an expression, where `await` is no keyword; an async function's result may await.
  const source = AWAIT.test(expression) && !FUNCTION.test(expression) ? `(async () => (${expression}))()` : expression
  const page = await context.browser.page()
  const result: unknown = await page.evaluate(source)
  context.print(result === undefined ? 'undefined' : JSON.stringify(result, null, 2))
}

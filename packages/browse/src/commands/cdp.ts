// `bun browse cdp <method> [params]`: sends one DevTools protocol command to
// the page, its parameters as JSON, and prints the answer.
import type { CDPSession } from 'playwright'
import { CommandFailure, parse, type Context } from '../command'

const USAGE = 'cdp <method> [params as JSON]'

/** Any of the protocol's methods, and any method's parameters. */
type Method = Parameters<CDPSession['send']>[0]
type Params = Parameters<CDPSession['send']>[1]

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const [method, params] = positionals
  if (method === undefined || positionals.length > 2) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  let parsed: unknown
  try {
    parsed = params === undefined ? undefined : JSON.parse(params)
  } catch (error) {
    throw new CommandFailure(`The parameters are not JSON: ${error instanceof Error ? error.message : error}`)
  }
  const session = await context.browser.cdp()
  // Playwright types `send` by the protocol's method names, and this one
  // comes from the command line: the browser answers a method or parameters
  // it does not know with an error, which is what the caller sees.
  const answer: unknown = await session.send(
    method as Method,
    parsed as Params,
  )
  context.print(JSON.stringify(answer, null, 2))
}

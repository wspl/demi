// `bun check stop`: closes the slot's browser and ends its daemon; the
// slot's servers keep running until `down`.
import { CheckFailure, parse, type Context } from '../command'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, 'stop')
  if (positionals.length > 0) {
    throw new CheckFailure('Usage: bun check stop')
  }
  context.endDaemon()
  context.print('Closing the browser')
}

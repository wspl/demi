// `bun browse stop`: closes the slot's browser and ends its daemon; the
// slot's servers keep running until `down`.
import { CommandFailure, parse, type Context } from '../command'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, 'stop')
  if (positionals.length > 0) {
    throw new CommandFailure('Usage: bun browse stop')
  }
  context.endDaemon()
  context.print('Closing the browser')
}

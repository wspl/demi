// `bun browse log console|network|sockets`: prints what the page logged,
// requested, or sent and received on its sockets since the last `log mark`,
// or since the tool attached to the browser with `--all`. `log network` leaves out the
// dev server's modules, styles and fonts unless `--modules`.
import { CommandFailure, parse, type Context } from '../command'
import { LOG_KINDS } from '../logs'

const USAGE = 'log console|network|sockets [--all] [--modules] | log mark [name]'
const OPTIONS = { all: { type: 'boolean' }, modules: { type: 'boolean' } } as const

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  const [kind, name] = positionals
  const logs = context.browser.logs
  if (kind === 'mark') {
    context.print(`Marked ${logs.mark(name ?? null)}`)
    return
  }
  const known = LOG_KINDS.find((entry) => entry === kind)
  if (!known || name !== undefined) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  if (!context.browser.running) {
    throw new CommandFailure('The tool is attached to no browser in this slot, so it logged nothing')
  }
  const all = values.all ?? false
  const lines = logs.read(known, { all, modules: values.modules ?? false })
  context.print(`${lines.length} ${known} entr${lines.length === 1 ? 'y' : 'ies'} ${logs.since(all)}`)
  for (const line of lines) {
    context.print(line)
  }
}

// `bun browse grant <permission>...`: grants the browser permissions, such as
// `clipboard` (read and write) or `local-network-access`, for every origin
// or the one `--origin` names; `--clear` takes every grant back.
import { CommandFailure, parse, type Context } from '../command'

const USAGE = 'grant <permission>... [--origin <url>] | grant --clear'
const OPTIONS = { origin: { type: 'string' }, clear: { type: 'boolean' } } as const

/** Names that stand for several of Playwright's permissions. */
const GROUPS: Record<string, string[]> = { clipboard: ['clipboard-read', 'clipboard-write'] }

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  const browserContext = (await context.browser.page()).context()
  if (values.clear) {
    if (positionals.length > 0) {
      throw new CommandFailure(`Usage: bun browse ${USAGE}`)
    }
    await browserContext.clearPermissions()
    context.print('Cleared every grant')
    return
  }
  if (positionals.length === 0) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const permissions = positionals.flatMap((name) => GROUPS[name] ?? [name])
  await browserContext.grantPermissions(permissions, values.origin === undefined ? undefined : { origin: values.origin })
  context.print(`Granted ${permissions.join(', ')} ${values.origin === undefined ? 'everywhere' : `to ${values.origin}`}`)
}

// `bun browse script <file> [args...]`: runs the default export of a module,
// for a step no command covers yet. It gets the page, its context, a CDP
// session, the arguments, and `print` and `shot` as the commands use them;
// a script that proves useful becomes a command.
import { writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { CommandFailure, parse, type Context } from '../command'
import { shotPath } from '../shots'

const USAGE = 'script <file> [args...]'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const [file, ...args] = positionals
  if (file === undefined) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  // A new query each time: the daemon would otherwise reuse the module it
  // loaded before the script changed.
  const module: unknown = await import(`${resolve(context.slot.root, file)}?run=${Date.now()}`)
  const main = typeof module === 'object' && module !== null && 'default' in module ? module.default : undefined
  if (typeof main !== 'function') {
    throw new CommandFailure(`${file} has no default export to run`)
  }
  const page = await context.browser.page()
  const result: unknown = await main({
    page,
    context: page.context(),
    cdp: await context.browser.cdp(),
    args,
    print: context.print,
    shot: async (name?: string) => {
      const path = shotPath(context.slot, name)
      writeFileSync(path, await context.browser.capture())
      context.print(path)
      return path
    },
  })
  if (result !== undefined) {
    context.print(JSON.stringify(result, null, 2))
  }
}

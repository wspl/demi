// `bun browse upload <locator> <file>...`: gives files to a file input, or,
// when the locator names a control such as Attach, to the file chooser its
// click opens.
import { resolve as resolvePath } from 'node:path'
import { CommandFailure, parse, type Context } from '../command'
import { element } from '../find'

const USAGE = 'upload <locator> <file>...'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const [locator, ...files] = positionals
  if (locator === undefined || files.length === 0) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const paths = files.map((file) => resolvePath(context.slot.root, file))
  const target = await element(context, locator)
  const isInput = await target.evaluate((node) => node instanceof HTMLInputElement && node.type === 'file')
  if (isInput) {
    await target.setInputFiles(paths)
  } else {
    const page = await context.browser.page()
    const [chooser] = await Promise.all([page.waitForEvent('filechooser', { timeout: 10_000 }), target.click()])
    await chooser.setFiles(paths)
  }
  context.print(`Gave ${paths.length} file${paths.length === 1 ? '' : 's'}`)
}

// `bun browse download '<command>'`: runs one command, such as a click on
// Save, and waits for the download it starts in the browser, which it keeps
// in `.cache/browse/downloads/` and names with its size: what the user's
// browser saved, which no screenshot shows.
import { mkdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { CommandFailure, parse, timeoutMs, timeoutOption, type Context } from '../command'
import { splitWords } from '../words'

const USAGE = 'download \'<command>\' [--timeout <s>]'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, timeoutOption, USAGE)
  if (positionals.length === 0) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const action = positionals.length === 1 ? splitWords(positionals[0]) : positionals
  const page = await context.browser.page()
  const timeout = timeoutMs(values.timeout, 30)
  // Listened for before the action, so a download it starts at once is not missed.
  const started = page.waitForEvent('download', { timeout }).catch(() => null)
  await context.run(action)
  const download = await started
  if (download === null) {
    throw new CommandFailure(`The command started no download within ${timeout / 1000} s`)
  }
  const failure = await download.failure()
  if (failure !== null) {
    throw new CommandFailure(`The download of ${download.url()} failed: ${failure}`)
  }
  const folder = join(context.slot.folder, 'downloads')
  mkdirSync(folder, { recursive: true })
  const path = join(folder, download.suggestedFilename())
  await download.saveAs(path)
  context.print(`${path}  ${statSync(path).size} bytes, from ${download.url()}`)
}

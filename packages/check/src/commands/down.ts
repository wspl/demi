// `bun check down` (product-checks.md § The slot's servers): stops exactly
// the process groups the slot's state records (the runner, the web app, the
// gallery, then the backend, which stops its Cloud as it ends), removes the
// runner's installation, deletes the `.env` that `up` copied, and closes the
// browser. Nothing found by name is ever stopped.
import { rmSync } from 'node:fs'
import { join } from 'node:path'
import { CheckFailure, parse, type Context } from '../command'
import { stopGroup } from '../processes'
import { slotPaths } from '../slot'
import { readState, updateState, type ServerName } from '../state'

/** The order of stopping: what uses the backend first. */
const ORDER: ServerName[] = ['runner', 'web', 'gallery', 'backend']

/** How long each may take to stop before it is killed; the backend hibernates the Cloud first. */
const GRACE_MS: Record<ServerName, number> = { runner: 15_000, web: 10_000, gallery: 10_000, backend: 30_000 }

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, 'down')
  if (positionals.length > 0) {
    throw new CheckFailure('Usage: bun check down')
  }
  const state = readState(context.slot)
  for (const name of ORDER) {
    const group = state.servers[name]
    if (!group) {
      continue
    }
    const outcome = await stopGroup(group, GRACE_MS[name])
    updateState(context.slot, (current) => {
      delete current.servers[name]
    })
    context.print(`${name.padEnd(8)} ${outcome}`)
  }
  rmSync(slotPaths(context.slot).runnerHome, { recursive: true, force: true })
  if (state.envCopied) {
    rmSync(join(context.slot.root, '.env'), { force: true })
    context.print('Deleted the .env up copied')
  }
  updateState(context.slot, (current) => {
    delete current.envCopied
    delete current.account
    delete current.reopen
  })
  context.endDaemon()
  context.print('Closing the browser')
}

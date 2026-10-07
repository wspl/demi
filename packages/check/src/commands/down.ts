// `bun check down [backend] [web] [gallery] [runner]` (product-checks.md
// § The slot's servers): stops exactly the process groups the slot's state
// records (the runner, the web app, the gallery, then the backend, which
// stops its Cloud as it ends), removes the runner's installation, deletes the
// `.env` that `up` copied, and closes the browser. Naming servers stops only
// those and keeps the rest, the browser and the account, as a check of a
// backend that goes away and comes back with `up backend` needs. Nothing
// found by name is ever stopped.
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

const USAGE = 'down [backend] [web] [gallery] [runner]'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const unknown = positionals.filter((name) => !ORDER.some((server) => server === name))
  if (unknown.length > 0) {
    throw new CheckFailure(`down stops ${ORDER.join(', ')}, not ${unknown.join(', ')}`)
  }
  const state = readState(context.slot)
  const named = positionals.length > 0
  for (const name of ORDER) {
    if (named && !positionals.includes(name)) {
      continue
    }
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
  if (named) {
    return
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

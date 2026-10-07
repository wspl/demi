// `bun browse down [backend] [web] [gallery] [runner] | down --wipe`
// (browse.md § The slot's servers): stops exactly the process groups the
// slot's state records (the runner, the web app, the gallery, then the
// backend, which stops its Cloud as it ends), deletes the `.env` that `up`
// copied, and stops the browser. Naming servers stops only those and keeps
// the rest, the browser and the account, as a check of a backend that goes
// away and comes back with `up backend` needs. The backend's data and the
// paired runner's installation stay for the next `up`; `--wipe` removes
// them too. Nothing found by name is ever stopped.
import { rmSync } from 'node:fs'
import { join } from 'node:path'
import { CommandFailure, parse, type Context } from '../command'
import { stopGroup } from '../processes'
import { slotPaths } from '../slot'
import { readState, updateState, type ServerName } from '../state'

/** The order of stopping: what uses the backend first. */
const ORDER: ServerName[] = ['runner', 'web', 'gallery', 'backend']

/** How long each may take to stop before it is killed; the backend hibernates the Cloud first. */
const GRACE_MS: Record<ServerName, number> = { runner: 15_000, web: 10_000, gallery: 10_000, backend: 30_000 }

const USAGE = 'down [backend] [web] [gallery] [runner] | down --wipe'
const OPTIONS = { wipe: { type: 'boolean' } } as const

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  const unknown = positionals.filter((name) => !ORDER.some((server) => server === name))
  if (unknown.length > 0) {
    throw new CommandFailure(`down stops ${ORDER.join(', ')}, not ${unknown.join(', ')}`)
  }
  const named = positionals.length > 0
  if (named && values.wipe) {
    throw new CommandFailure(`--wipe stops everything first, so it names no server\nUsage: bun browse ${USAGE}`)
  }
  const state = readState(context.slot)
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
  if (state.envCopied) {
    rmSync(join(context.slot.root, '.env'), { force: true })
    context.print('Deleted the .env up copied')
  }
  updateState(context.slot, (current) => {
    delete current.envCopied
    delete current.account
  })
  if (values.wipe) {
    wipe(context)
  }
  context.endDaemon()
  context.print('Closing the browser')
}

/** Removes the backend's data and the paired runner, whose device that data held. */
function wipe(context: Context): void {
  const paths = slotPaths(context.slot)
  rmSync(paths.backendData, { recursive: true, force: true })
  rmSync(paths.runnerHome, { recursive: true, force: true })
  updateState(context.slot, (current) => {
    delete current.runner
  })
  context.print(`Removed the backend's data ${paths.backendData} and the runner's installation`)
}

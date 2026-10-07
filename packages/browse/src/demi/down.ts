// `demi.down(...servers)` and `demi.down({ wipe: true })` (browse.md § What
// `demi` adds): stops exactly the process groups the slot's state records
// (the runner, the web app, the gallery, then the backend, which stops its
// Cloud as it ends), deletes the `.env` that `demi.up` copied, and closes
// the browser. Naming servers stops only those and keeps the rest, the
// browser and the account, as a check of a backend that goes away and comes
// back with `demi.up('backend')` needs. The backend's data and the paired
// runner's installation stay for the next `demi.up`; `wipe` removes them
// too. Nothing found by name is ever stopped.
import { rmSync } from 'node:fs'
import { join } from 'node:path'
import { stopGroup } from '../processes'
import { slotPaths } from '../slot'
import { readState, updateState, type ServerName } from '../state'
import { Failure, type Tool } from '../tool'

/** The order of stopping: what uses the backend first. */
const ORDER: ServerName[] = ['runner', 'web', 'gallery', 'backend']

/** How long each may take to stop before it is killed; the backend hibernates the Cloud first. */
const GRACE_MS: Record<ServerName, number> = { runner: 15_000, web: 10_000, gallery: 10_000, backend: 30_000 }

export type DownArgument = string | { wipe?: boolean }

export async function down(tool: Tool, args: DownArgument[]): Promise<void> {
  const named = args.filter((arg) => typeof arg === 'string')
  const wipe = args.some((arg) => typeof arg === 'object' && arg !== null && arg.wipe === true)
  const unknown = named.filter((name) => !ORDER.some((server) => server === name))
  if (unknown.length > 0) {
    throw new Failure(`demi.down stops ${ORDER.join(', ')}, not ${unknown.join(', ')}`)
  }
  if (named.length > 0 && wipe) {
    throw new Failure('demi.down({ wipe: true }) stops everything first, so it names no server')
  }
  const state = readState(tool.slot)
  for (const name of ORDER) {
    if (named.length > 0 && !named.includes(name)) {
      continue
    }
    const group = state.servers[name]
    if (!group) {
      continue
    }
    const outcome = await stopGroup(group, GRACE_MS[name])
    updateState(tool.slot, (current) => {
      delete current.servers[name]
    })
    tool.print(`${name.padEnd(9)} ${outcome}`)
  }
  if (named.length > 0) {
    return
  }
  if (state.envCopied) {
    rmSync(join(tool.slot.root, '.env'), { force: true })
    tool.print('Deleted the .env demi.up copied')
  }
  updateState(tool.slot, (current) => {
    delete current.envCopied
    delete current.account
  })
  if (wipe) {
    removeData(tool)
  }
  await tool.browser.close()
  tool.endServer()
  tool.print('Closed the browser')
}

/** Removes the backend's data and the paired runner, whose device that data held. */
function removeData(tool: Tool): void {
  const paths = slotPaths(tool.slot)
  rmSync(paths.backendData, { recursive: true, force: true })
  rmSync(paths.runnerHome, { recursive: true, force: true })
  updateState(tool.slot, (current) => {
    delete current.runner
  })
  tool.print(`Removed the backend's data ${paths.backendData} and the runner's installation`)
}

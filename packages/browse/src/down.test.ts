// `down` on a slot folder in a temporary directory, with no process
// recorded, so it stops nothing; a few milliseconds.
import { afterEach, expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Browser } from './browser'
import type { Context } from './command'
import { run } from './commands/down'
import { slotPaths, slotPorts, type Slot } from './slot'
import { readState, updateState } from './state'

const folders: string[] = []

afterEach(() => {
  for (const folder of folders.splice(0)) {
    rmSync(folder, { recursive: true, force: true })
  }
})

/** A slot whose backend has data, whose runner is paired, and which took a screenshot. */
function slotWithData(): Slot {
  const root = mkdtempSync(join(tmpdir(), 'browse-down-'))
  folders.push(root)
  const slot: Slot = { root, number: 3, ports: slotPorts(3), folder: join(root, '.cache/browse') }
  const paths = slotPaths(slot)
  for (const folder of [paths.backendData, paths.runnerHome, paths.shots]) {
    mkdirSync(folder, { recursive: true })
  }
  writeFileSync(join(paths.backendData, 'demi.db'), '')
  updateState(slot, (state) => {
    state.runner = { generation: 1, device: 'browse-slot-3' }
  })
  return slot
}

function contextOf(slot: Slot): Context & { ended: () => boolean } {
  let ended = false
  return {
    slot,
    browser: new Browser(slot, () => undefined),
    network: () => Promise.reject(new Error('down needs no network')),
    print: () => undefined,
    env: {},
    run: () => Promise.reject(new Error('down runs no other command')),
    endDaemon: () => {
      ended = true
    },
    ended: () => ended,
  }
}

test('the backend\'s data and the paired runner outlive down, for the next up to come back to', async () => {
  const slot = slotWithData()
  const context = contextOf(slot)
  await run(context, [])
  const paths = slotPaths(slot)
  expect([existsSync(join(paths.backendData, 'demi.db')), existsSync(paths.runnerHome), readState(slot).runner?.device]).toEqual([true, true, 'browse-slot-3'])
  expect(context.ended()).toBe(true)
})

test('down --wipe removes the backend\'s data and the runner it paired, and keeps the screenshots', async () => {
  const slot = slotWithData()
  await run(contextOf(slot), ['--wipe'])
  const paths = slotPaths(slot)
  expect([existsSync(paths.backendData), existsSync(paths.runnerHome), readState(slot).runner, existsSync(paths.shots)]).toEqual([false, false, undefined, true])
})

test('down --wipe stops everything, so it takes no server\'s name', async () => {
  const slot = slotWithData()
  await expect(run(contextOf(slot), ['backend', '--wipe'])).rejects.toThrow('--wipe stops everything first')
  expect(existsSync(slotPaths(slot).backendData)).toBe(true)
})

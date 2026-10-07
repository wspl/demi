// `demi.runner({ fresh })`, `demi.runner.stop()` and `demi.runner.start()`
// (browse.md § What `demi` adds): the slot's runner, a runner of the slot's
// build for the slot's backend, a device of its own with its installation in
// the slot's folder. The first `demi.runner()` pairs it the way a person
// does: Settings › Devices › Add Device, the pairing code the runner printed,
// Pair Device. `stop` stops it, as a device that goes away, and `start`, or
// `demi.runner()` again, starts the same runner on the same installation,
// which keeps the device; `fresh` pairs a new one in its place. `demi.down`
// stops it and keeps it paired; `demi.down({ wipe: true })` removes it.
import { existsSync, mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { startGroup, stopGroup, type Started } from '../processes'
import { runnerName, runnerStep, type RunnerRequest } from '../runners'
import { childEnv, freshLog, lineOf, runningServer, untilReady } from '../servers'
import { local, slotPaths } from '../slot'
import { readState, updateState } from '../state'
import { Failure, webBase, type Tool } from '../tool'

/** What the runner prints to its console (`demi-runner-protocol`'s `console` module). */
const PAIRING_CODE = /^demi-runner: pairing code: (\S+)$/m
/** Printed once a runner is paired, and by a paired runner each time it starts and connects. */
const PAIRED = /^demi-runner: paired as (.+)$/m

/** How long the runner may take to print a code, to hear it is paired, and to connect. */
const RUNNER_MS = 60_000
/** How long the runner may take to stop before it is killed. */
const STOP_MS = 15_000

export function runner(tool: Tool) {
  return Object.assign(
    (options: { fresh?: boolean } = {}) => act(tool, options.fresh ? 'new' : 'default'),
    {
      stop: () => act(tool, 'stop'),
      start: () => act(tool, 'start'),
    },
  )
}

/** Does what `request` asks of the slot's runner; answers the device's name once it runs, or null once it is stopped. */
async function act(tool: Tool, request: RunnerRequest): Promise<string | null> {
  const { slot } = tool
  const state = readState(slot)
  const running = runningServer(slot, 'runner')
  const step = runnerStep(request, state.runner, running !== undefined)
  const device = state.runner?.device ?? null
  switch (step.kind) {
    case 'already running':
      return device
    case 'not running':
      return null
    case 'nothing paired':
      throw new Failure('The slot has no paired runner to start; demi.runner() pairs one')
    case 'stop': {
      const outcome = running === undefined ? 'not running' : await stopGroup(running, STOP_MS)
      updateState(slot, (current) => {
        delete current.servers.runner
      })
      tool.print(`Runner    ${device ?? 'the slot\'s runner'} ${outcome}; the backend shows the device offline`)
      return null
    }
    case 'start': {
      requireServers(tool)
      const generation = state.runner?.generation ?? 1
      const started = start(tool, generation)
      const connected = await untilReady('The runner', started, async () => lineOf(started.group.log, PAIRED)?.[1] ?? null, RUNNER_MS)
      tool.print(`Runner    ${connected} started again and connected; its log is ${started.group.log}`)
      return connected
    }
    case 'pair':
      requireServers(tool)
      if (step.stop && running !== undefined) {
        await stopGroup(running, STOP_MS)
        tool.print(device === null
          ? 'Runner    stopped the slot\'s runner, whose pairing never ended, to pair it again'
          : `Runner    ${device} stopped; the backend keeps it as an offline device`)
      }
      return pair(tool, step.generation)
  }
}

function requireServers(tool: Tool): void {
  if (!runningServer(tool.slot, 'backend') || !runningServer(tool.slot, 'web')) {
    throw new Failure('The runner connects to the backend and pairs through the web app: call demi.up() first')
  }
}

/** Starts the runner of `generation` on the slot's installation and records its group. */
function start(tool: Tool, generation: number): Started {
  const { slot } = tool
  const program = join(slot.root, 'target/debug/demi-runner')
  if (!existsSync(program)) {
    throw new Failure(`${program} is missing; demi.up() builds it with the backend`)
  }
  const home = slotPaths(slot).runnerHome
  const command = [program, 'run', '--backend', local(slot.ports.backend), '--home', home, '--name', runnerName(slot.number, generation)]
  const started = startGroup(command, { cwd: slot.root, env: childEnv(tool, {}), log: freshLog(slot, 'runner') })
  updateState(slot, (state) => {
    state.servers.runner = started.group
  })
  return started
}

/** Pairs a runner of `generation` in a new installation through Add Device; answers its device name. */
async function pair(tool: Tool, generation: number): Promise<string> {
  const { slot } = tool
  const home = slotPaths(slot).runnerHome
  rmSync(home, { recursive: true, force: true })
  mkdirSync(home, { recursive: true })
  updateState(slot, (state) => {
    state.runner = { generation, device: null }
  })
  const started = start(tool, generation)
  const code = await untilReady('The runner', started, async () => lineOf(started.group.log, PAIRING_CODE)?.[1] ?? null, RUNNER_MS)
  tool.print(`Runner    ${runnerName(slot.number, generation)} started; pairing code ${code}`)

  const page = await tool.browser.page()
  const back = page.url()
  // Settings have addresses of their own (`web-application.md` § Authentication).
  await page.goto(`${webBase(slot)}/settings/devices`)
  // Each step is a locator, which Playwright finds again for each attempt:
  // a button the dialog replaces while it is clicked, as Continue is, is
  // clicked once it is there again.
  await page.getByRole('button', { name: 'Add Device…' }).click()
  const dialog = page.getByRole('dialog', { name: 'Add Device' })
  await dialog.getByRole('button', { name: 'Continue' }).click()
  await dialog.getByLabel('Pairing code').fill(code)
  await dialog.getByRole('button', { name: 'Pair Device' }).click()
  await dialog.getByRole('button', { name: 'Done' }).click({ timeout: RUNNER_MS })
  const device = await untilReady('The runner', started, async () => lineOf(started.group.log, PAIRED)?.[1] ?? null, RUNNER_MS)
  updateState(slot, (state) => {
    state.runner = { generation, device }
  })
  await page.keyboard.press('Escape')
  if (page.url() !== back && back.startsWith(webBase(slot))) {
    await page.goto(back)
  }
  tool.print(`Runner    paired as ${device}; its log is ${started.group.log}`)
  return device
}

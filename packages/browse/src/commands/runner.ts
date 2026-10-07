// `bun browse runner [stop | start] [--new]` (browse.md § The real
// product's other parts): the slot's runner, a runner of the slot's build
// for the slot's backend, a device of its own with its installation in the
// slot's folder. The first `runner` pairs it the way a person does:
// Settings › Devices › Add Device, the pairing code the runner printed,
// Pair Device. `runner stop` stops it, as a device that goes away, and
// `runner start`, or `runner` again, starts the same runner on the same
// installation, which keeps the device; `runner --new` pairs a new one in
// its place. `down` stops it and keeps it paired; `down --wipe` removes it.
import { existsSync, mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { CommandFailure, parse, webBase, type Context } from '../command'
import { element } from '../find'
import { startGroup, stopGroup, type Started } from '../processes'
import { runnerName, runnerStep, type RunnerRequest } from '../runners'
import { childEnv, freshLog, lineOf, runningServer, untilReady } from '../servers'
import { local, slotPaths } from '../slot'
import { readState, updateState } from '../state'

const USAGE = 'runner [stop | start] [--new]'
const OPTIONS = { new: { type: 'boolean' } } as const

/** What the runner prints to its console (`demi-runner-protocol`'s `console` module). */
const PAIRING_CODE = /^demi-runner: pairing code: (\S+)$/m
/** Printed once a runner is paired, and by a paired runner each time it starts and connects. */
const PAIRED = /^demi-runner: paired as (.+)$/m

/** How long the runner may take to print a code, to hear it is paired, and to connect. */
const RUNNER_MS = 60_000
/** How long the runner may take to stop before it is killed. */
const STOP_MS = 15_000

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals, values } = parse(argv, OPTIONS, USAGE)
  const [action] = positionals
  if (positionals.length > 1 || (action !== undefined && action !== 'stop' && action !== 'start') || (values.new && action !== undefined)) {
    throw new CommandFailure(`Usage: bun browse ${USAGE}`)
  }
  const request: RunnerRequest = values.new ? 'new' : action ?? 'default'
  const { slot } = context
  const state = readState(slot)
  const running = runningServer(slot, 'runner')
  const step = runnerStep(request, state.runner, running !== undefined)
  const device = state.runner?.device ?? 'the slot\'s runner'
  switch (step.kind) {
    case 'already running':
      context.print(`Runner ${device} already runs; bun browse runner stop stops it`)
      return
    case 'not running':
      context.print('The slot\'s runner does not run')
      return
    case 'nothing paired':
      throw new CommandFailure('The slot has no paired runner to start; bun browse runner pairs one')
    case 'stop': {
      const outcome = running === undefined ? 'not running' : await stopGroup(running, STOP_MS)
      updateState(slot, (current) => {
        delete current.servers.runner
      })
      context.print(`Runner ${device} ${outcome}; the backend shows the device offline`)
      return
    }
    case 'start': {
      requireServers(context)
      const generation = state.runner?.generation ?? 1
      const started = start(context, generation)
      const connected = await untilReady('The runner', started, async () => lineOf(started.group.log, PAIRED)?.[1] ?? null, RUNNER_MS)
      context.print(`Runner ${connected} started again and connected; its log is ${started.group.log}`)
      return
    }
    case 'pair':
      requireServers(context)
      if (step.stop && running !== undefined) {
        await stopGroup(running, STOP_MS)
        context.print(`Stopped runner ${device}; the backend keeps it as an offline device`)
      }
      await pair(context, step.generation)
  }
}

function requireServers(context: Context): void {
  if (!runningServer(context.slot, 'backend') || !runningServer(context.slot, 'web')) {
    throw new CommandFailure('The runner connects to the backend and pairs through the web app: run bun browse up first')
  }
}

/** Starts the runner of `generation` on the slot's installation and records its group. */
function start(context: Context, generation: number): Started {
  const { slot } = context
  const program = join(slot.root, 'target/debug/demi-runner')
  if (!existsSync(program)) {
    throw new CommandFailure(`${program} is missing; bun browse up builds it with the backend`)
  }
  const home = slotPaths(slot).runnerHome
  const command = [program, 'run', '--backend', local(slot.ports.backend), '--home', home, '--name', runnerName(slot.number, generation)]
  const started = startGroup(command, { cwd: slot.root, env: childEnv(context, {}), log: freshLog(slot, 'runner') })
  updateState(slot, (state) => {
    state.servers.runner = started.group
  })
  return started
}

/** Pairs a runner of `generation` in a new installation through Add Device. */
async function pair(context: Context, generation: number): Promise<void> {
  const { slot } = context
  const home = slotPaths(slot).runnerHome
  rmSync(home, { recursive: true, force: true })
  mkdirSync(home, { recursive: true })
  updateState(slot, (state) => {
    state.runner = { generation, device: null }
  })
  const started = start(context, generation)
  const code = await untilReady('The runner', started, async () => lineOf(started.group.log, PAIRING_CODE)?.[1] ?? null, RUNNER_MS)
  context.print(`Runner ${runnerName(slot.number, generation)} started; pairing code ${code}`)

  const page = await context.browser.page()
  const back = page.url()
  // Settings have addresses of their own (`web-application.md` § Authentication).
  await page.goto(`${webBase(slot)}/settings/devices`)
  await (await element(context, 'role=button[name="Add Device…"]', 10_000)).click()
  await (await element(context, 'role=dialog[name="Add Device"] >> role=button[name="Continue"]')).click()
  await (await element(context, 'role=dialog[name="Add Device"] >> label=Pairing code')).fill(code)
  await (await element(context, 'role=dialog[name="Add Device"] >> role=button[name="Pair Device"]')).click()
  await (await element(context, 'role=dialog[name="Add Device"] >> role=button[name="Done"]', RUNNER_MS)).click()
  const device = await untilReady('The runner', started, async () => lineOf(started.group.log, PAIRED)?.[1] ?? null, RUNNER_MS)
  updateState(slot, (state) => {
    state.runner = { generation, device }
  })
  await page.keyboard.press('Escape')
  if (page.url() !== back && back.startsWith(webBase(slot))) {
    await page.goto(back)
  }
  context.print(`Paired as ${device}; its log is ${started.group.log}`)
}

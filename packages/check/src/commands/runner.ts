// `bun check runner` (product-checks.md § The real product's other parts):
// starts a runner of the slot's build for the slot's backend, as a device
// of its own with its installation in the slot's folder, and pairs it the
// way a person does: Settings › Devices › Add Device, the pairing code the
// runner printed, Pair Device. `down` stops the runner and removes its
// installation.
import { existsSync, mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { CheckFailure, parse, webBase, type Context } from '../command'
import { element } from '../find'
import { startGroup } from '../processes'
import { childEnv, freshLog, lineOf, runningServer, untilReady } from '../servers'
import { local, slotPaths } from '../slot'
import { updateState } from '../state'

/** What the runner prints to its console (`demi-runner-protocol`'s `console` module). */
const PAIRING_CODE = /^demi-runner: pairing code: (\S+)$/m
const PAIRED = /^demi-runner: paired as (.+)$/m

/** How long the runner may take to print a code, and to hear it is paired. */
const RUNNER_MS = 60_000

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, 'runner')
  if (positionals.length > 0) {
    throw new CheckFailure('Usage: bun check runner')
  }
  const { slot } = context
  if (!runningServer(slot, 'backend') || !runningServer(slot, 'web')) {
    throw new CheckFailure('The runner pairs through the web app: run bun check up first')
  }
  if (runningServer(slot, 'runner')) {
    context.print('The slot\'s runner already runs; bun check down stops it')
    return
  }
  const program = join(slot.root, 'target/debug/demi-runner')
  if (!existsSync(program)) {
    throw new CheckFailure(`${program} is missing; bun check up builds it with the backend`)
  }
  const home = slotPaths(slot).runnerHome
  rmSync(home, { recursive: true, force: true })
  mkdirSync(home, { recursive: true })
  const name = `check-slot-${slot.number}`
  const command = [program, 'run', '--backend', local(slot.ports.backend), '--home', home, '--name', name]
  const started = startGroup(command, { cwd: slot.root, env: childEnv(context, {}), log: freshLog(slot, 'runner') })
  updateState(slot, (state) => {
    state.servers.runner = started.group
  })
  const code = await untilReady('The runner', started, async () => lineOf(started.group.log, PAIRING_CODE)?.[1] ?? null, RUNNER_MS)
  context.print(`Runner ${name} started; pairing code ${code}`)

  const page = await context.browser.page()
  const back = page.url()
  if (!back.startsWith(webBase(slot))) {
    await page.goto(`${webBase(slot)}/`)
  }
  await page.keyboard.press('ControlOrMeta+Comma')
  await (await element(context, 'role=navigation[name="Settings sections"] >> role=button[name="Devices"]', 10_000)).click()
  await (await element(context, 'role=button[name="Add Device"]')).click()
  await (await element(context, 'role=dialog[name="Add Device"] >> role=button[name="Continue"]')).click()
  await (await element(context, 'role=dialog[name="Add Device"] >> label=Pairing code')).fill(code)
  await (await element(context, 'role=dialog[name="Add Device"] >> role=button[name="Pair Device"]')).click()
  await (await element(context, 'role=dialog[name="Add Device"] >> role=button[name="Done"]', RUNNER_MS)).click()
  const paired = await untilReady('The runner', started, async () => lineOf(started.group.log, PAIRED)?.[1] ?? null, RUNNER_MS)
  await page.keyboard.press('Escape')
  if (page.url() !== back && back.startsWith(webBase(slot))) {
    await page.goto(back)
  }
  context.print(`Paired as ${paired}; its log is ${started.group.log}`)
}

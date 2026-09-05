import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { HOSTLESS_ENV } from '../../conversation/hostless-shell'
import { LocalControlService, type ControlService } from '../../storage/control'
import { openSqliteDatabase } from '../../storage/database'
import { FakeProvisioner } from './fake-provisioner'
import { itemsText } from './model'
import { World } from './world'
import { model, type Driver } from './driver'

// S11 — the session upgrade (`sessions-and-targets.md` § Hostless
// execution): the first script outside tinybash's subset moves a hostless
// conversation to a machine of its own, files and shell state included,
// and runs there whole; nothing enters the transcript. Then split
// equivalence: the same script sequence run whole on a machine and split
// at every point, tool results and final files compared byte for byte.

let world: World
let fake: FakeProvisioner
let control: ControlService
let controlDb: ReturnType<typeof openSqliteDatabase>

beforeAll(async () => {
  fake = new FakeProvisioner()
  world = await World.create({ managedHosts: { provisioner: fake, config: { hostsPerUser: 20, sweepMs: 60_000 } }, providerRequestsPerMinute: 10_000 })
  controlDb = openSqliteDatabase(join(world.dataDir, 'control.sqlite'))
  control = new LocalControlService(controlDb)
})

afterAll(async () => {
  try {
    await world.close()
  } finally {
    await fake.close()
    controlDb.close()
  }
})

/** A command outside the subset that prints nothing: what forces the upgrade without leaving a trace in the results. */
const TRIGGER = 'uname > /dev/null'

test('the first outside script moves the conversation to a machine, silently, with its files and shell state', async () => {
  const driver = await world.conversation('hostless')
  const owner = { kind: 'conversation' as const, id: driver.id }
  const before = await driver.turn({
    model: [model.shell('t1', "mkdir -p work && cd work && printf 'alpha\\n' > a.txt && MARK=set && printf 'tmp\\n' > /tmp/scratch.txt && echo ok"), model.say('ready')],
  })
  expect(before.received[0]).toContain('ok')
  expect(await world.hostlessFile(driver.id, '/home/demi/work/a.txt')).toBe('alpha\n')

  // `$(…)` is outside the grammar: the whole script runs on the machine, from the directory and with the variable tinybash held.
  const upgraded = await driver.turn({ model: [model.shell('t2', 'echo "$MARK in $(pwd)"; cat a.txt; demi host current'), model.say('moved')] })
  const home = fake.homeOf(owner)
  expect(upgraded.received[0]).toContain(`set in ${home}/work`)
  expect(upgraded.received[0]).toContain('alpha')
  expect(upgraded.received[0]).toContain('host: machine "cloud"')
  expect(itemsText(upgraded.requests[0]!.items)).not.toContain('[Execution target switched]')
  expect(driver.transcript().some((block) => block.type === 'user' && block.preamble)).toBe(false)
  const record = await control.getConversation(driver.id)
  expect(record?.hostDeviceId).not.toBeNull()
  expect(record?.pendingSwitch).toBeNull()
  // The files moved: the tree is empty, the home holds them with /tmp under .tmp.
  expect(await world.hostlessFile(driver.id, '/home/demi/work/a.txt')).toBeNull()
  expect(await readFile(join(home, 'work', 'a.txt'), 'utf8')).toBe('alpha\n')
  expect(await readFile(join(home, '.tmp', 'scratch.txt'), 'utf8')).toBe('tmp\n')

  // From then on every call runs there; the machine's shell carries its cwd.
  const after = await driver.turn({ model: [model.shell('t3', 'printf beta >> a.txt && cat a.txt && echo "$PATH|$LANG"'), model.say('done')] })
  expect(after.received[0]).toContain('alpha\nbeta')
  // The machine's jobs run with the hostless environment table: the same `$PATH` the model saw before the upgrade.
  expect(after.received[0]).toContain(`${HOSTLESS_ENV.PATH}|${HOSTLESS_ENV.LANG}`)
  const starts = world.wire().filter((f) => f.deviceId === record!.hostDeviceId && f.message.type === 'job_start').map((f) => (f.message.type === 'job_start' ? f.message.env : {}))
  expect(starts.length).toBeGreaterThan(0)
  expect(starts.every((env) => env.PATH === HOSTLESS_ENV.PATH && env.SHELL === HOSTLESS_ENV.SHELL)).toBe(true)
  expect(fake.calls.filter((call) => call.startsWith('provision:'))).toHaveLength(1)
}, 60_000)

test('a provisioning failure is that call\'s tool error; the conversation stays hostless with its files', async () => {
  const limited = new FakeProvisioner()
  const small = await World.create({ managedHosts: { provisioner: limited, config: { hostsPerUser: 0, sweepMs: 60_000 } } })
  try {
    const driver = await small.conversation('hostless')
    await driver.turn({ model: [model.shell('t1', "printf keep > k.txt"), model.say('ok')] })
    const failed = await driver.turn({ model: [model.shell('t2', 'uname'), model.say('failed')] })
    expect(failed.received[0]).toContain('limit of 0 machines')
    const still = await driver.turn({ model: [model.shell('t3', 'cat k.txt && demi host current'), model.say('still')] })
    expect(still.received[0]).toContain('keep')
    expect(still.received[0]).toContain('host: virtual')
    expect(limited.calls).toEqual([])
  } finally {
    await small.close()
    await limited.close()
  }
}, 60_000)

/** Commands inside the subset whose output is the same under BSD and GNU coreutils. */
const SEQUENCE = [
  "mkdir -p src && printf 'one\\ntwo\\n' > src/a.txt && echo made",
  "cd src && cat a.txt && printf 'three\\n' >> a.txt",
  'grep -c o a.txt',
  'sed -n 2p a.txt',
  "demi file create b.md <<'EOF'\nhello\nworld\nEOF",
  'cat b.md a.txt | head -n 3',
  'ls',
]

/** Runs the sequence with the upgrade forced before step `split` (0 = everything on the machine). */
async function run(split: number): Promise<{ driver: Driver; results: string[] }> {
  const driver = await world.conversation('hostless')
  const results: string[] = []
  for (let step = 0; step <= SEQUENCE.length; step += 1) {
    if (step === split) await driver.turn({ model: [model.shell(`u${step}`, TRIGGER), model.say('trigger')] })
    if (step === SEQUENCE.length) break
    const turn = await driver.turn({ model: [model.shell(`s${step}`, SEQUENCE[step]!), model.say(`step ${step}`)] })
    results.push(turn.received[0]!)
  }
  return { driver, results }
}

test('split equivalence: the sequence whole on the machine equals every split with the first part hostless', async () => {
  const whole = await run(0)
  const wholeHome = fake.homeOf({ kind: 'conversation', id: whole.driver.id })
  const wholeFiles = { a: await readFile(join(wholeHome, 'src', 'a.txt'), 'utf8'), b: await readFile(join(wholeHome, 'src', 'b.md'), 'utf8') }
  expect(whole.results[2]).toContain('preview:\n2\n')
  for (let split = 1; split <= SEQUENCE.length; split += 1) {
    const part = await run(split)
    expect(part.results, `split at ${split}`).toEqual(whole.results)
    const home = fake.homeOf({ kind: 'conversation', id: part.driver.id })
    expect(await readFile(join(home, 'src', 'a.txt'), 'utf8'), `a.txt at split ${split}`).toBe(wholeFiles.a)
    expect(await readFile(join(home, 'src', 'b.md'), 'utf8'), `b.md at split ${split}`).toBe(wholeFiles.b)
  }
}, 180_000)

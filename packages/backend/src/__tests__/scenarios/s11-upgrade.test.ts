import { deferred, delay } from '@demicodes/utils'
import { mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { HOSTLESS_ENV } from '../../conversation/hostless-shell'
import { ownerKey, type BootArgs } from '../../managed/provisioner'
import { LocalControlService, type ControlService, type ManagedHostOwner } from '../../storage/control'
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
  world = await World.create({ runners: ['alpha'], managedHosts: { provisioner: fake, config: { hostsPerUser: 20, sweepMs: 60_000 } }, providerRequestsPerMinute: 10_000 })
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
  expect(record?.lastSwitch).toBeNull()
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
    const refusedSpawn = await driver.turn({ model: [model.shell('t4', "printf wrong > k.txt; demi agent spawn 'work'"), model.say('refused')] })
    expect(refusedSpawn.received[0]).toContain('limit of 0 machines')
    expect(await small.hostlessFile(driver.id, '/home/demi/k.txt')).toBe('keep')
    const help = await driver.turn({ model: [model.shell('t5', 'demi agent spawn --help'), model.say('help')] })
    expect(help.received[0]).toContain('Start an isolated child')
    expect(limited.calls).toEqual([])
  } finally {
    await small.close()
    await limited.close()
  }
}, 60_000)

test('the shells the model holds continue on the machine: a named shell under its id, in its directory, with its variables; the default stays the default', async () => {
  const driver = await world.conversation('hostless')
  const owner: ManagedHostOwner = { kind: 'conversation', id: driver.id }
  const first = await driver.turn({ model: [model.shell('n1', 'mkdir -p named && cd named && TOKEN=kept && echo in-named'), model.say('one')] })
  // The shell's id as the frames carry it (a short result does not expose the handle to the model).
  const shellId = first.shell.at(-1)!.shellId
  // The outside script names the shell: it runs on the machine under that id, where the shell stood, with what it set.
  const named = await driver.turn({ model: [model.tool('n2', 'shell_exec', { script: 'echo "$TOKEN in $(pwd)"', timeoutMs: 10_000, shellId }), model.say('two')] })
  const home = fake.homeOf(owner)
  expect(named.received[0]).toContain(`kept in ${home}/named`)
  expect(named.shell.at(-1)!.shellId).toBe(shellId)
  // Without a shellId the session's default is still that shell; with it, a later call still finds it.
  const byDefault = await driver.turn({ model: [model.shell('n3', 'pwd'), model.say('three')] })
  expect(byDefault.received[0]).toContain(`${home}/named\n`)
  expect(byDefault.shell.at(-1)!.shellId).toBe(shellId)
  const again = await driver.turn({ model: [model.tool('n4', 'shell_exec', { script: 'echo "still $TOKEN"; cd ..', timeoutMs: 10_000, shellId }), model.say('four')] })
  expect(again.received[0]).toContain('still kept')
  const moved = await driver.turn({ model: [model.shell('n5', 'pwd'), model.say('five')] })
  expect(moved.received[0]).toContain(`${home}\n`)
}, 60_000)

test("spawn acquires the machine before the parent script runs; parent and child share its files", async () => {
  const driver = await world.conversation('hostless')
  const owner: ManagedHostOwner = { kind: 'conversation', id: driver.id }
  await driver.turn({ model: [model.shell('p1', 'mkdir -p shared && cd shared && printf parent > p.txt && echo ok'), model.say('ready')] })
  world.model.scriptChild(model.shell('c1', 'printf child > c.txt; echo "child in $(pwd)"'), model.say('child done'))
  const spawned = await driver.turn({ model: [model.shell('p2', "demi agent spawn 'write c.txt' --description writer; cat ../c.txt", 20_000), model.say('spawned')] })
  expect(spawned.received[0]).toContain('child done')
  expect(spawned.received[0]).toContain('child')
  expect(spawned.received[0]).not.toContain('No such file')
  const home = fake.homeOf(owner)
  const childRequests = world.model.requests.filter((request) => request.sessionId !== driver.id)
  expect(itemsText(childRequests.at(-1)!.items)).toContain(`child in ${home}`)
  expect((await control.getConversation(driver.id))?.hostDeviceId).not.toBeNull()
  // The parent's default shell was left in `shared` hostless; on the machine it is there, and both files are where each wrote them.
  const after = await driver.turn({ model: [model.shell('p3', 'pwd; cat p.txt ../c.txt; demi host current'), model.say('done')] })
  expect(after.received[0]).toContain(`${home}/shared\n`)
  expect(after.received[0]).toContain('parentchild')
  expect(after.received[0]).toContain('host: machine "cloud"')
  expect(fake.calls.filter((call) => call === `provision:${ownerKey(owner)}`)).toHaveLength(1)
}, 60_000)

/** A provisioner whose first boot fails after the device row exists — what a full backend machine looks like from above. */
class FlakyProvisioner extends FakeProvisioner {
  failuresLeft = 1

  override async provision(owner: ManagedHostOwner, homeDir: string, boot: BootArgs): Promise<void> {
    if (this.failuresLeft > 0) {
      this.failuresLeft -= 1
      this.calls.push(`provision-failed:${ownerKey(owner)}`)
      throw new Error('no capacity right now')
    }
    return super.provision(owner, homeDir, boot)
  }
}

test('a failed upgrade leaves nothing behind, and the retry moves the tree as it stands then', async () => {
  const flaky = new FlakyProvisioner()
  const small = await World.create({ managedHosts: { provisioner: flaky, config: { hostsPerUser: 20, sweepMs: 60_000 } } })
  const smallDb = openSqliteDatabase(join(small.dataDir, 'control.sqlite'))
  const smallControl = new LocalControlService(smallDb)
  try {
    const driver = await small.conversation('hostless')
    const owner: ManagedHostOwner = { kind: 'conversation', id: driver.id }
    await driver.turn({ model: [model.shell('f1', 'printf one > one.txt'), model.say('one')] })
    const failed = await driver.turn({ model: [model.shell('f2', 'uname'), model.say('failed')] })
    expect(failed.received[0]).toContain('no capacity right now')
    // No device row, no binding, the files still in the tree.
    expect(await smallControl.getManagedDevice(owner)).toBeNull()
    expect((await smallControl.getConversation(driver.id))?.hostDeviceId).toBeNull()
    expect(await small.hostlessFile(driver.id, '/home/demi/one.txt')).toBe('one')
    await driver.turn({ model: [model.shell('f3', 'printf two > two.txt'), model.say('two')] })
    const moved = await driver.turn({ model: [model.shell('f4', 'uname > /dev/null; cat one.txt two.txt'), model.say('moved')] })
    expect(moved.received[0]).toContain('onetwo')
    expect(await readFile(join(flaky.homeOf(owner), 'two.txt'), 'utf8')).toBe('two')
    expect(flaky.calls.filter((call) => call.startsWith('provision'))).toEqual([`provision-failed:${ownerKey(owner)}`, `provision:${ownerKey(owner)}`])
  } finally {
    await small.close()
    await flaky.close()
    smallDb.close()
  }
}, 60_000)

test('a conversation with a machine of its own never returns to hostless, from the machine or from a workspace', async () => {
  const driver = await world.conversation('hostless')
  await driver.turn({ model: [model.shell('l1', TRIGGER), model.say('moved')] })
  const patch = (body: unknown) => world.backend.session.fetch(`/api/conversations/${driver.id}`, { method: 'PATCH', body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })
  const fromMachine = await patch({ workspaceId: null })
  expect(fromMachine.status).toBe(409)
  expect(((await fromMachine.json()) as { code: string }).code).toBe('no_hostless_entrance')
  // To a workspace: allowed, the machine becomes an attached host. Back to hostless from there: still refused.
  const workspaceId = world.device('alpha').workspaceId
  expect((await patch({ workspaceId })).status).toBe(200)
  const fromWorkspace = await patch({ workspaceId: null })
  expect(fromWorkspace.status).toBe(409)
  expect(((await fromWorkspace.json()) as { code: string }).code).toBe('no_hostless_entrance')
  const record = await control.getConversation(driver.id)
  expect(record?.workspaceId).toBe(workspaceId)
  expect((await control.listAttachedHosts(driver.id)).map((host) => host.name)).toEqual(['cloud'])
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

class PausedProvisioner extends FakeProvisioner {
  readonly entered = deferred<void>()
  readonly proceed = deferred<void>()
  override async provision(owner: ManagedHostOwner, homeDir: string, boot: BootArgs): Promise<void> {
    this.entered.resolve()
    await this.proceed.promise
    return super.provision(owner, homeDir, boot)
  }
}

test('an upload arriving after materialization waits and writes to the committed machine', async () => {
  const paused = new PausedProvisioner()
  const small = await World.create({ managedHosts: { provisioner: paused } })
  try {
    const driver = await small.conversation('hostless')
    await driver.upload('before.txt', new TextEncoder().encode('before'))
    const turn = driver.startTurn({ model: [model.shell('upgrade', 'uname > /dev/null'), model.say('ready')] })
    await paused.entered.promise
    let uploaded = false
    const upload = driver.upload('during.txt', new TextEncoder().encode('during')).then(() => { uploaded = true })
    await delay(20)
    expect(uploaded).toBe(false)
    expect(await small.hostlessFile(driver.id, '/home/demi/before.txt')).toBe('before')
    paused.proceed.resolve()
    await Promise.all([turn.done, upload])
    const home = paused.homeOf({ kind: 'conversation', id: driver.id })
    expect(await readFile(join(home, 'before.txt'), 'utf8')).toBe('before')
    expect(await readFile(join(home, 'during.txt'), 'utf8')).toBe('during')
    expect(await small.hostlessFile(driver.id, '/home/demi/during.txt')).toBeNull()
  } finally {
    paused.proceed.resolve()
    await small.close()
    await paused.close()
  }
}, 60_000)

for (const state of ['prepared', 'committed'] as const) {
  test(`restart recovers a ${state} cutover from its durable record`, async () => {
    const provisioner = new FakeProvisioner()
    const small = await World.create({ port: 0, managedHosts: { provisioner } })
    const db = openSqliteDatabase(join(small.dataDir, 'control.sqlite'))
    const store = new LocalControlService(db)
    try {
      const driver = await small.conversation('hostless')
      await driver.upload('source.txt', new TextEncoder().encode('source'))
      const owner: ManagedHostOwner = { kind: 'conversation', id: driver.id }
      const home = await mkdtemp(join(tmpdir(), 'demi-recovery-home-'))
      await writeFile(join(home, 'source.txt'), 'source')
      await store.beginUpgrade(driver.id)
      const device = await small.backend.managedHosts!.provisionFresh(owner, small.backend.session.user.id, home)
      if (state === 'committed') expect(await store.bindConversationHost(driver.id, device.id)).toBe(true)
      expect(await store.listUpgrades()).toEqual([{ conversationId: driver.id, state }])
      await small.restartBackend()
      expect(await store.listUpgrades()).toEqual([])
      if (state === 'prepared') {
        expect(await store.getManagedDevice(owner)).toBeNull()
        expect(await small.hostlessFile(driver.id, '/home/demi/source.txt')).toBe('source')
      } else {
        expect((await store.getConversation(driver.id))?.hostDeviceId).toBe(device.id)
        expect(await small.hostlessFile(driver.id, '/home/demi/source.txt')).toBeNull()
        await driver.upload('next.txt', new TextEncoder().encode('next'))
        expect(await readFile(join(home, 'source.txt'), 'utf8')).toBe('source')
        expect(await readFile(join(home, 'next.txt'), 'utf8')).toBe('next')
      }
    } finally {
      db.close()
      await small.close()
      await provisioner.close()
    }
  }, 60_000)
}

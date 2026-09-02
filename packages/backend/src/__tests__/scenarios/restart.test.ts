import { readFile } from 'node:fs/promises'
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { delay, waitFor } from '@demicodes/utils'
import { World } from './world'
import { model } from './driver'

// R1–R4 — the restarts. The world has a fixed port so a runner finds the
// restarted backend on its own; a runner is stopped and started over its own
// state directory.

let world: World

beforeAll(async () => {
  const probe = Bun.serve({ port: 0, fetch: () => new Response('') })
  const port = probe.port ?? 0
  probe.stop(true)
  world = await World.create({ runners: ['alpha'], port })
})

afterAll(async () => {
  await world.close()
})

test('R1 — backend restart, idle: the runner reconnects, the conversation resumes on it, the ledger carries over', async () => {
  const driver = await world.conversation('runner:alpha')
  const before = await driver.turn({ model: [model.shell('t1', 'echo -n kept > kept.txt && cat kept.txt'), model.say('remember me')] })
  expect(before.received[0]).toContain('kept')
  const blocksBefore = driver.transcript().map((block) => block.id)
  const usageBefore = await world.api<{ totals: Array<{ requests: number }> }>('/api/usage')

  await world.restartBackend()
  await driver.attach()
  await waitFor(() => driver.transcript().length === blocksBefore.length, () => `${driver.transcript().length} blocks`, { timeoutMs: 10_000 })
  expect(driver.transcript().map((block) => block.id)).toEqual(blocksBefore)

  const after = await driver.turn({ model: [model.shell('t2', 'cat kept.txt && demi host current'), model.say('and again')] })
  expect(after.received[0]).toContain('kept')
  expect(after.received[0]).toContain('on device "alpha"')
  const usageAfter = await world.api<{ totals: Array<{ requests: number }> }>('/api/usage')
  const sum = (usage: { totals: Array<{ requests: number }> }) => usage.totals.reduce((total, group) => total + group.requests, 0)
  expect(sum(usageAfter)).toBe(sum(usageBefore) + 2)
}, 60_000)

test('R2 — backend restart mid-turn: the transcript has no dangling tool call; the next turn executes', async () => {
  const driver = await world.conversation('runner:alpha')
  await driver.turn({ model: [model.say('warm')] })
  const started = world.frames.length
  const { begin } = driver.startTurn({ model: [model.shell('t1', 'sleep 5; echo late', 10_000), model.say('never')] })
  await waitFor(() => world.frames.slice(started).some((frame) => frame.message.type === 'job_start'), undefined, { timeoutMs: 5_000 })
  await delay(100)

  world.wire()
  await world.restartBackend()
  await driver.attach()
  await waitFor(() => driver.transcript().length > 0, undefined, { timeoutMs: 10_000 })
  // The verdict (`progress.md`): closing the backend aborts the turn — the job
  // is killed on the runner, the tool call is settled as an error, and the
  // turn closes with an abort block. Nothing dangles.
  const cut = driver.transcript().slice(begin.blocks).map((block) => (block.type === 'tool_call' ? `${block.type}:${block.status}` : block.type))
  expect(cut).toEqual(['user', 'tool_call:error', 'response', 'abort'])
  const kills = world.wire('alpha').map((frame) => `${frame.direction}:${frame.message.type}`)
  expect(kills).toContain('out:job_kill')
  expect(kills).toContain('in:job_exit')
  // The model never got to answer that turn; its script goes with it.
  world.model.clear(driver.id)

  const next = await driver.turn({ model: [model.shell('t2', 'echo after restart'), model.say('recovered')] })
  expect(next.received[0]).toContain('Tool call aborted')
  expect(next.received[1]).toContain('after restart')
  expect(driver.lastText()).toBe('recovered')
}, 60_000)

test('R3 — runner death mid-command is a tool error; the returned runner serves the next turn; files survive', async () => {
  const driver = await world.conversation('runner:alpha')
  await driver.turn({ model: [model.shell('t1', 'echo -n before > before.txt'), model.say('written')] })

  const started = world.frames.length
  const { begin, done } = driver.startTurn({ model: [model.shell('t2', 'sleep 5; echo late', 10_000), model.say('the runner is gone')] })
  await waitFor(() => world.frames.slice(started).some((frame) => frame.message.type === 'job_start'), undefined, { timeoutMs: 5_000 })
  await world.killRunner('alpha')
  await done
  // The loss reaches the model as the command's end: exit 127, named.
  const killed = driver.observe(begin)
  expect(killed.received[0]).toContain('exitCode: 127')
  expect(killed.received[0]).toContain('runner disconnected')
  expect(driver.lastText()).toBe('the runner is gone')

  await world.returnRunner('alpha')
  const back = await driver.turn({ model: [model.shell('t3', 'cat before.txt'), model.say('back')] })
  expect(back.received[0]).toContain('before')
  expect(await readFile(driver.filePath('before.txt'), 'utf8')).toBe('before')
}, 60_000)

test('R4 — hostless persistence: files, todos and the ledger survive a backend restart', async () => {
  const driver = await world.conversation('hostless')
  const first = await driver.turn({
    model: [model.shell('t1', "demi file create notes.md <<'EOF'\nkeep me\nEOF\ndemi todo add \"still here\""), model.say('stored')],
  })
  expect(first.received[0]).toContain('Created notes.md')
  const usageBefore = await world.api<{ totals: Array<{ requests: number }> }>('/api/usage')

  await world.restartBackend()
  await driver.attach()
  await waitFor(() => driver.transcript().length > 0, undefined, { timeoutMs: 10_000 })
  const after = await driver.turn({ model: [model.shell('t2', 'cat notes.md && demi todo list'), model.say('found')] })
  expect(after.received[0]).toContain('keep me')
  expect(after.received[0]).toContain('still here')
  const usageAfter = await world.api<{ totals: Array<{ requests: number }> }>('/api/usage')
  const sum = (usage: { totals: Array<{ requests: number }> }) => usage.totals.reduce((total, group) => total + group.requests, 0)
  expect(sum(usageAfter)).toBe(sum(usageBefore) + 2)
}, 60_000)

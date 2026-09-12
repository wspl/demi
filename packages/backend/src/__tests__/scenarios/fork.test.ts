import { afterAll, beforeAll, expect, test } from 'bun:test'
import { deferred, waitFor } from '@demicodes/utils'
import type { Block } from '@demicodes/core'
import { events } from '@demicodes/provider/testing'
import type { ConversationRecord } from '../../storage/control'
import { login } from '../session'
import { World } from './world'
import { Driver, model } from './driver'

let world: World
beforeAll(async () => {
  const probe = Bun.serve({ port: 0, fetch: () => new Response('') })
  const port = probe.port!
  probe.stop(true)
  world = await World.create({ port, providerRequestsPerMinute: 1000 })
})
afterAll(async () => { await world.close() })

const fork = (sourceId: string, blockId: string, id: string = crypto.randomUUID()) =>
  world.api<{ conversation: ConversationRecord }>(`/api/conversations/${sourceId}/fork`, { id, blockId })

test('Fork keeps the exact prefix and historical todos while the source runs, with durable idempotent creation', async () => {
  const source = await world.conversation('cloud')
  await world.api(`/api/conversations/${source.id}`, { title: 'Planning' }, 'PATCH')
  await source.turn({ text: 'first', model: [
    model.shell('add', 'echo shared > shared.txt && demi todo add "first task"'),
    model.say('first answer'),
  ] })
  const answer = source.transcript().find((block) => block.type === 'text' && block.text === 'first answer')!
  const prefix = structuredClone(source.transcript().slice(0, source.transcript().findIndex((block) => block.id === answer.id) + 1))
  await source.turn({ text: 'second', model: [model.shell('done', 'demi todo done T1'), model.say('second answer')] })
  const laterAnswer = source.transcript().filter((block) => block.type === 'text').at(-1)!
  const streamed = deferred<void>()
  const release = deferred<void>()
  const running = source.turn({ text: 'running', model: [async function* () {
    yield events.text('unfinished')
    streamed.resolve()
    await release.promise
    yield events.text(' complete')
    yield events.response()
  }] })
  await streamed.promise
  try {
    const requests = world.model.requests.length
    const id = crypto.randomUUID()
    const created = (await fork(source.id, answer.id, id)).conversation
    expect(created).toMatchObject({ id, title: 'Planning (Fork)', archived: false, pinned: false })
    expect(created.target.kind).toBe('cloud')
    if (created.target.kind === 'cloud') expect(created.target.path).toContain(source.id)
    expect(source.events.filter((event) => event.type === 'phase').at(-1)?.phase).toBe('running')
    expect(world.model.requests).toHaveLength(requests)
    const cold = await world.api<{ blocks: Block[]; subagents: unknown[] }>(`/api/conversations/${id}/transcript`)
    expect(cold.blocks).toEqual(prefix)
    expect(cold.subagents).toEqual([])
    expect((await fork(source.id, answer.id, id)).conversation.id).toBe(id)
    await expect(fork(source.id, laterAnswer.id, id)).rejects.toThrow('409')
    const unfinished = source.transcript().filter((block) => block.type === 'text').at(-1)!
    await expect(fork(source.id, unfinished.id)).rejects.toThrow('400')
    const branch = await Driver.attachExisting(world, id, 'cloud')
    expect(branch.transcript()).toEqual(prefix)
    const checked = await branch.turn({ text: 'check', model: [
      model.shell('check', 'cat shared.txt && demi todo list --json'), model.say('branch checked'),
    ] })
    expect(checked.received.at(-1)).toContain('shared')
    expect(checked.received.at(-1)).toContain('"status":"pending"')
    expect(source.events.filter((event) => event.type === 'phase').at(-1)?.phase).toBe('running')
    const later = (await fork(source.id, laterAnswer.id)).conversation
    const laterBranch = await Driver.attachExisting(world, later.id, 'cloud')
    const laterChecked = await laterBranch.turn({ model: [model.shell('check-later', 'demi todo list --json'), model.say('later')] })
    expect(laterChecked.received.at(-1)).toContain('"status":"done"')
    const nested = (await fork(id, answer.id)).conversation
    expect(nested.title).toBe('Planning (Fork) (Fork)')
    const beforeRestart = structuredClone(branch.transcript())
    release.resolve()
    await running
    await world.restartBackend()
    await branch.attach()
    await waitFor(() => branch.agent.transcriptVersion() !== null)
    expect(branch.transcript()).toEqual(beforeRestart)
    expect((await fork(source.id, answer.id, id)).conversation.id).toBe(id)
    const afterRestart = await branch.turn({ model: [model.shell('after-restart', 'cat shared.txt && demi todo list --json'), model.say('reopened')] })
    expect(afterRestart.received.at(-1)).toContain('shared')
    expect(afterRestart.received.at(-1)).toContain('"status":"pending"')
  } finally {
    release.resolve()
    await running
  }
}, 60_000)

test('a source child continues only in the source tree and its existing references survive Fork', async () => {
  const source = await world.conversation('cloud')
  const childStarted = deferred<void>()
  const releaseChild = deferred<void>()
  world.model.scriptChild(async function* () {
    childStarted.resolve()
    await releaseChild.promise
    yield* model.say('child result belongs to source')
  })
  try {
    await source.turn({ model: [
      model.shell('spawn', "demi agent spawn <<< 'wait for release' --description worker", 20),
      model.say('child is still working'),
    ] })
    await childStarted.promise
    const answer = source.transcript().filter((block) => block.type === 'text').at(-1)!
    const created = (await fork(source.id, answer.id)).conversation
    const before = await world.api<{ blocks: Block[]; subagents: unknown[] }>(`/api/conversations/${created.id}/transcript`)
    expect(before.blocks.some((block) => block.type === 'tool_call')).toBe(true)
    expect(before.subagents).toEqual([])
    source.script(model.say('source received its child result'))
    releaseChild.resolve()
    await waitFor(() => source.events.some((event) => event.type === 'subagent' && event.event === 'closed'))
    const after = await world.api<{ blocks: Block[]; subagents: unknown[] }>(`/api/conversations/${created.id}/transcript`)
    expect(after).toEqual(before)
    const branch = await Driver.attachExisting(world, created.id, 'cloud')
    const checked = await branch.turn({ model: [model.shell('children', 'demi agent list'), model.say('empty tree')] })
    expect(checked.received.at(-1)).not.toContain('worker')
  } finally {
    releaseChild.resolve()
  }
}, 60_000)

test('Fork rejects other owners, malformed bodies and ordinary conversation UUIDs', async () => {
  const source = await world.conversation('cloud')
  await source.turn({ model: [model.say('owned')] })
  const answer = source.transcript().find((block) => block.type === 'text')!
  const other = await world.conversation('cloud')
  await expect(fork(source.id, answer.id, other.id)).rejects.toThrow('409')
  await world.api('/api/users', { email: 'fork-other@example.test', password: 'other-pass-123', role: 'user' })
  const actor = await login(world.backend, 'fork-other@example.test', 'other-pass-123')
  const response = await actor.fetch(`/api/conversations/${source.id}/fork`, {
    method: 'POST', body: JSON.stringify({ id: crypto.randomUUID(), blockId: answer.id }),
    headers: { 'content-type': 'application/json' },
  })
  expect(response.status).toBe(404)
  const malformed = await world.backend.session.fetch(`/api/conversations/${source.id}/fork`, {
    method: 'POST', body: JSON.stringify({ id: 'invalid', blockId: answer.id, transcript: [] }),
    headers: { 'content-type': 'application/json' },
  })
  expect(malformed.status).toBe(400)
})

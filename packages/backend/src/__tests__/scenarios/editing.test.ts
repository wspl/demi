import { afterAll, beforeAll, expect, test } from 'bun:test'
import { deferred, waitFor } from '@demicodes/utils'
import type { Block } from '@demicodes/core'
import type { EditRequest } from '@demicodes/agent'
import { World } from './world'
import { model } from './driver'
import { itemsText } from './model'

let world: World
beforeAll(async () => {
  const probe = Bun.serve({ port: 0, fetch: () => new Response('') })
  const port = probe.port!
  probe.stop(true)
  world = await World.create({ port, providerRequestsPerMinute: 1000 })
})
afterAll(async () => { await world.close() })

test('authenticated edit restores todos, preserves files, and reconciles across takeover and restart', async () => {
  const driver = await world.conversation('cloud')
  await driver.turn({ text: 'A-kept', model: [model.say('answer-A-kept')] })
  const effects = await driver.turn({ text: 'B-removed', model: [
    model.shell('external-effect', 'echo -n permanent > sentinel.txt && demi todo add "permanent todo"'),
    model.say('answer-B-removed'),
  ] })
  expect(effects.received[0]).toContain('permanent todo')
  expect(await driver.readFile('sentinel.txt')).toBe('permanent')
  await driver.turn({ text: 'C-removed', model: [model.say('answer-C-removed')] })
  const before = structuredClone(driver.transcript())
  const target = before.find((block) => block.type === 'user' && block.content.some((part) =>
    part.type === 'text' && part.text === 'B-removed',
  ))!
  const request: EditRequest = {
    operationId: crypto.randomUUID(), targetBlockId: target.id,
    version: driver.agent.transcriptVersion()!,
    content: [
      { type: 'text', text: 'B-edited\nmultiline' },
      { type: 'image', source: { type: 'binary', data: new Uint8Array([7, 8]), mediaType: 'image/png' } },
      { type: 'text', text: 'last text' },
    ],
  }
  const stale = { ...structuredClone(request), operationId: crypto.randomUUID() }
  const start = world.model.requests.length
  const entered = deferred<void>()
  const release = deferred<void>()
  driver.script(async function* (input) {
    expect(itemsText(input.items)).toContain('A-kept')
    expect(itemsText(input.items)).toContain('answer-A-kept')
    expect(itemsText(input.items)).not.toContain('B-removed')
    expect(itemsText(input.items)).not.toContain('C-removed')
    expect(input.items.some((item) => item.type === 'tool_result')).toBe(false)
    const last = input.items.at(-1)
    expect(last?.type).toBe('user_message')
    if (last?.type === 'user_message') expect(last.content.slice(-3)).toEqual(request.content)
    entered.resolve()
    await release.promise
    yield* model.say('answer-edited')
  })
  try {
    await driver.agent.editAndSend(request)
    await entered.promise
    const targetIndex = before.findIndex((block) => block.id === target.id)
    expect(driver.transcript().slice(0, targetIndex)).toEqual(before.slice(0, targetIndex))
    expect(driver.transcript().slice(targetIndex).map((block) => block.type)).toEqual(['user'])
    await driver.detach()
    await driver.attach()
    await driver.agent.editAndSend(request)
    await expect(driver.agent.editAndSend(stale)).rejects.toThrow()
    expect(world.model.requests.length).toBe(start + 1)
  } finally {
    release.resolve()
  }
  await waitFor(() => driver.lastText() === 'answer-edited')
  const cold = await world.api<{ blocks: Block[] }>(`/api/conversations/${driver.id}/transcript`)
  expect(cold.blocks.map((block) => block.id)).toEqual(driver.transcript().map((block) => block.id))
  expect(await driver.readFile('sentinel.txt')).toBe('permanent')
  const accepted = structuredClone(driver.transcript())
  await world.restartBackend()
  await driver.attach()
  await waitFor(() => driver.agent.transcriptVersion() !== null)
  expect(driver.transcript()).toEqual(accepted)
  await driver.agent.editAndSend(request)
  expect(world.model.requests.length).toBe(start + 1)
  await expect(driver.agent.editAndSend(stale)).rejects.toThrow('conversation changed')
  await driver.turn({ text: 'verify effects', model: [
    model.shell('inspect-effects', 'cat sentinel.txt && demi todo list'),
    model.say('checked'),
  ] }).then((turn) => {
    expect(turn.received.at(-1)).toContain('permanentNo todos.')
    expect(turn.received.at(-1)).not.toContain('permanent todo')
  })
}, 60_000)

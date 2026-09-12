import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import type { Block } from '@demicodes/core'
import { FakeProvisioner } from './fake-provisioner'
import { World } from './world'
import { itemsText } from './model'
import { model, type Target } from './driver'

// S5 — inherited subagents share the parent target and can read and write.
// The parent receives completion messages and subagent lifecycle frames.

const fake = new FakeProvisioner()
let world: World

beforeAll(async () => {
  world = await World.create({
    runners: ['alpha'],
    managedHosts: {
      provisioner: fake,
      config: { maxRunning: 30 },
    },
  })
})

afterAll(async () => {
  await world.close()
  await fake.close()
})

describe.each<Target>(['cloud', 'runner:alpha'])('S5 subagents on %s', (target) => {
  test("inherited children read and write the parent's files", async () => {
    const driver = await world.conversation(target)
    await driver.turn({
      model: [
        model.shell(
          't1',
          "demi file create notes.md <<'EOF'\nthe answer is 42\nEOF",
        ),
        model.say('written'),
      ],
    })

    // The first child: sees only its brief and works on the same target as its parent.
    let brief = ''
    world.model.scriptChild(
      (request) => {
        brief = itemsText(request.items)
        return model.shell('c1', 'cat notes.md')
      },
      model.shell('c2', "demi file create blocked.md <<'EOF'\nnope\nEOF"),
      model.say('the file says 42; I wrote too'),
    )
    const explored = await driver.turn({
      model: [
        model.shell(
          't2',
          "demi agent spawn <<< 'Read notes.md and report its content' --description reader",
          10_000,
        ),
        model.say('reader dispatched'),
        model.say('explored'),
      ],
    })
    await waitFor(() => driver.lastText() === 'explored')
    expect(brief).toContain('Read notes.md and report its content')
    expect(brief).not.toContain('written')
    const childRequests = world.model.requests.filter(
      (request) => request.sessionId !== driver.id,
    )
    const childSaw = itemsText(childRequests.at(-1)!.items)
    expect(childSaw).toContain('the answer is 42')
    expect(childSaw).toContain('Created blocked.md')
    expect(explored.received[0]).toContain('subagentId:')
    expect(explored.received[0]).not.toContain('the file says 42; I wrote too')
    expect(await driver.readFile('blocked.md')).toBe('nope\n')

    // The second child writes where the parent then reads.
    world.model.scriptChild(
      model.shell('c3', "demi file create reply.md <<'EOF'\nfrom the child\nEOF"),
      model.say('wrote reply.md'),
    )
    const delegated = await driver.turn({
      model: [
        model.shell(
          't3',
          "demi agent spawn <<< 'Create reply.md' --description writer",
          10_000,
        ),
        model.say('writer dispatched'),
        model.shell('t4', 'cat reply.md'),
        model.say('delegated'),
      ],
    })
    await waitFor(() => driver.lastText() === 'delegated')
    expect(delegated.received[0]).toContain('subagentId:')
    expect(delegated.received[0]).not.toContain('wrote reply.md')
    expect(await driver.readFile('reply.md')).toBe('from the child\n')

    // The parent's stream carried the subagent lifecycle for both children.
    const lifecycle = driver.events
      .filter((event) => event.type === 'subagent')
      .map((event) => (event.type === 'subagent' ? event.event : ''))
    expect(lifecycle).toEqual(['started', 'closed', 'started', 'closed'])

    const history = await world.api<{
      subagents: {
        id: string
        name: string
        phase: string
        startedAt: string
        endedAt: string | null
        blocks: Block[]
      }[]
    }>(`/api/conversations/${driver.id}/transcript`)
    expect(history.subagents.map((agent) => agent.name).sort()).toEqual([
      'reader',
      'writer',
    ])
    for (const agent of history.subagents) {
      expect(agent.phase).toBe('completed')
      expect(Date.parse(agent.startedAt)).toBeGreaterThan(0)
      expect(Date.parse(agent.endedAt!)).toBeGreaterThanOrEqual(
        Date.parse(agent.startedAt),
      )
      expect(agent.blocks.some((block) => block.type === 'text')).toBe(true)
    }
  }, 60_000)
})

test('a cross-host command preserves the child node storage scope', async () => {
  const driver = await world.conversation('runner:alpha')
  await driver.turn({
    model: [
      model.shell('scope-root', 'demi todo add root-only'),
      model.say('ready'),
    ],
  })
  world.model.scriptChild(
    model.shell(
      'scope-remote',
      'demi host shell --host alpha "demi todo add child-only"',
    ),
    model.shell('scope-child-list', 'demi todo list'),
    model.say('child done'),
  )
  const parent = await driver.turn({
    model: [
      model.shell('scope-spawn', "demi agent spawn <<< 'add a todo on alpha'"),
      model.shell('scope-root-list', 'demi todo list'),
      model.say('parent idle'),
      model.say('child complete'),
    ],
  })
  await waitFor(() => driver.lastText() === 'child complete')
  expect(parent.received.at(-1)).toContain('root-only')
  expect(parent.received.at(-1)).not.toContain('child-only')
  const child = world.model.requests
    .filter(
      (request) =>
        request.sessionId !== driver.id &&
        request.items.some(
          (item) =>
            item.type === 'tool_result' && item.toolUseId === 'scope-child-list',
        ),
    )
    .at(-1)
  expect(itemsText(child!.items)).toContain('child-only')
})

test('a silent child runs past the HTTP idle timeout after spawn has exited', async () => {
  const driver = await world.conversation('runner:alpha')
  world.model.scriptChild(
    model.shell('child-pwd', 'pwd'),
    model.slowSay('long child completed', 14_000),
  )
  const startedAt = Date.now()
  const turn = await driver.turn({ model: [
    model.shell('spawn-long', "demi agent spawn --request-id long-child <<< 'quiet long task'", 2_000),
    model.say('creation accepted'),
    model.say('long result received'),
  ] })
  expect(Date.now() - startedAt).toBeLessThan(5_000)
  expect(turn.received[0]).toContain('status: exited')
  expect(turn.received[0]).toContain('exitCode: 0')
  expect(turn.received[0]).not.toContain('long child completed')
  expect(driver.events.some(event => event.type === 'subagent' && event.event === 'closed')).toBe(false)
  await waitFor(() => driver.lastText() === 'long result received', undefined, { timeoutMs: 20_000 })
  const closed = driver.events.filter(event => event.type === 'subagent' && event.event === 'closed')
  expect(closed).toHaveLength(1)
  expect(closed[0]?.type === 'subagent' && closed[0].job.phase).toBe('completed')
  expect(itemsText(world.model.requests.filter(request => request.sessionId === driver.id).at(-1)!.items)).toContain('long child completed')
}, 30_000)

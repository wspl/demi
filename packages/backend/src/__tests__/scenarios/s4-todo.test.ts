import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { World } from './world'
import { model, type Target } from './driver'

// S4 — `demi todo`: written in one turn, read in a later one; a second
// conversation sees an empty list. The rpc leaf crosses the relay on a
// runner and runs in-process hostless; storage is scoped to the session.

let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'] })
})

afterAll(async () => {
  await world.close()
})

describe.each<Target>(['hostless', 'runner:alpha'])('S4 todo on %s', (target) => {
  test('todos persist across turns and stay with their session', async () => {
    const driver = await world.conversation(target)
    world.wire()
    const added = await driver.turn({
      model: [model.shell('t1', 'demi todo add "draft the outline" && demi todo add "run the suite"'), model.say('added')],
    })
    expect(added.received[0]).toContain('exitCode: 0')

    const listed = await driver.turn({ model: [model.shell('t2', 'demi todo list --json'), model.say('listed')] })
    const json = /preview:\n([\s\S]*?)\nnext:/.exec(listed.received[0]!)?.[1] ?? /preview:\n([\s\S]*)$/.exec(listed.received[0]!)?.[1] ?? ''
    expect(JSON.parse(json.trim())).toMatchObject({ todos: [{ text: 'draft the outline' }, { text: 'run the suite' }] })

    // On a runner the leaf is relayed as rpc; hostless runs it in this process.
    const relayed = world.wire('alpha').filter((f) => f.message.type === 'rpc_call').map((f) => (f.message.type === 'rpc_call' ? f.message.path.join(' ') : ''))
    if (target === 'hostless') expect(relayed).toEqual([])
    else expect(relayed).toEqual(['demi todo add', 'demi todo add', 'demi todo list'])

    const other = await world.conversation(target)
    const empty = await other.turn({ model: [model.shell('t1', 'demi todo list --json'), model.say('empty')] })
    expect(empty.received[0]).toContain('{"todos":[]}')
  }, 30_000)
})

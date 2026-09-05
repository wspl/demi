import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { delay, waitFor } from '@demicodes/utils'
import { World } from './world'
import { model, type Target } from './driver'

// S9 — the client drops its socket while a turn runs; a new client attaches.
// The turn completes server-side, the reattached client's transcript has the
// result, and cold equals live (the teardown equality).

let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'] })
})

afterAll(async () => {
  await world.close()
})

describe.each<Target>(['hostless', 'runner:alpha'])('S9 detach on %s', (target) => {
  test('a turn survives its client', async () => {
    const driver = await world.conversation(target)
    await driver.turn({ model: [model.say('warm')] })

    // A command runs, then the model answers slowly; the socket drops in between.
    const { begin } = driver.startTurn({
      model: [model.shell('t1', 'echo -n survived > proof.txt && cat proof.txt'), model.slowSay('the complete slow answer arrived intact', 600)],
    })
    await delay(150)
    await driver.detach()
    await driver.attach()
    await waitFor(() => driver.lastText() === 'the complete slow answer arrived intact', () => driver.lastText(), { timeoutMs: 10_000 })

    const turn = driver.observe(begin)
    expect(turn.received[0]).toContain('survived')
    const types = driver.transcript().filter(block => !(block.type === 'user' && block.preamble?.startsWith('[Execution context '))).map((block) => block.type)
    expect(types).toEqual(['user', 'text', 'response', 'user', 'tool_call', 'response', 'text', 'response'])
  }, 30_000)
})

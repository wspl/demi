import { afterAll, beforeAll, expect, test } from 'bun:test'
import { World } from './world'
import { model } from './driver'

// S7 — two conversations on the same device, their commands interleaved:
// each session sees only its own working directory and shell state, and the
// job frames carry the right session attribution.

let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'] })
})

afterAll(async () => {
  await world.close()
})

test('two sessions on one runner keep their cwd and state apart', async () => {
  const a = await world.conversation('runner:alpha')
  const b = await world.conversation('runner:alpha')
  world.wire()

  // A moves into a directory and sets a variable while B, in parallel, does neither.
  const [first, second] = await Promise.all([
    a.turn({ model: [model.shell('a1', 'mkdir -p sub && cd sub && MARK=from-a && echo "a: $(pwd) $MARK"'), model.say('a moved')] }),
    b.turn({ model: [model.shell('b1', 'echo "b: $(pwd) mark=${MARK:-unset}"'), model.say('b stayed')] }),
  ])
  expect(first.received[0]).toContain(`a: ${a.filePath('sub')} from-a`)
  expect(second.received[0]).toContain(`b: ${b.filePath('')}`.replace(/\/$/, ''))
  expect(second.received[0]).toContain('mark=unset')

  // The next turns, interleaved again: A is still in sub (a machine's shell
  // carries its cwd between jobs and nothing else); B is not.
  world.model.scriptChild(model.slowSay('child of a', 400))
  const [third, fourth] = await Promise.all([
    a.turn({ model: [model.shell('a2', "demi agent spawn 'wait' --description w && echo \"a: $(pwd)\""), model.say('a again')] }),
    b.turn({ model: [model.shell('b2', 'echo "b: $(pwd) mark=${MARK:-unset}"'), model.say('b again')] }),
  ])
  expect(third.received[0]).toContain(`a: ${a.filePath('sub')}`)
  expect(fourth.received[0]).toContain(`b: ${b.filePath('')}`.replace(/\/$/, ''))
  expect(fourth.received[0]).toContain('mark=unset')

  // Every job on the wire names the session it ran for.
  const starts = world.wire('alpha').filter((f) => f.message.type === 'job_start').map((f) => (f.message.type === 'job_start' ? f.message.env.DEMI_SESSION_ID : ''))
  expect(starts.sort()).toEqual([a.id, a.id, b.id, b.id].sort())
}, 30_000)

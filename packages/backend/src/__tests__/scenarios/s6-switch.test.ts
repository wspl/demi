import { afterAll, beforeAll, expect, test } from 'bun:test'
import { FakeProvisioner } from './fake-provisioner'
import { World } from './world'
import { itemsText } from './model'
import { model } from './driver'

// S6 — continuing across a switch: a file made hostless, the conversation
// rebound to a runner over PATCH and worked there, then rebound back. The
// script keeps working across both switches, the context block appears at
// each, files are where each target keeps them, and the departed runner is
// granted to the conversation.

const fake = new FakeProvisioner()
let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'], managedHosts: { provisioner: fake, config: { hostsPerUser: 30 } } })
})

afterAll(async () => {
  await world.close()
  await fake.close()
})

test('hostless → runner → hostless, files staying with their target', async () => {
  const driver = await world.conversation('hostless')
  const created = await driver.turn({
    model: [model.shell('t1', "demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF"), model.say('created')],
  })
  expect(created.received[0]).toContain('Created notes.md')

  // To the runner: the next turn opens with the context block; the file is not here.
  await driver.switchTo('runner:alpha')
  const moved = await driver.turn({
    model: [model.shell('t2', 'cat notes.md; echo exit=$?'), model.shell('t3', "demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF"), model.say('recreated')],
  })
  expect(itemsText(moved.requests[0]!.items)).toContain('[Execution target switched]')
  expect(itemsText(moved.requests[0]!.items)).toContain('Previous target: the virtual environment')
  expect(moved.received[0]).toContain('No such file or directory')
  expect(moved.received[0]).toContain('exit=1')
  expect(moved.received[1]).toContain('Created notes.md')

  const edited = await driver.turn({
    model: [model.shell('t4', 'demi file edit notes.md --old beta --new delta && cat notes.md'), model.say('edited')],
  })
  expect(edited.received[0]).toContain('alpha\ndelta\ngamma')
  expect(await driver.readFile('notes.md')).toBe('alpha\ndelta\ngamma\n')
  expect(await world.hostlessFile(driver.id, '/home/demi/notes.md')).toBe('alpha\nbeta\ngamma\n')

  // Back to hostless: the original is untouched; the switch attached the runner under its name, so `host shell --host` reaches it.
  await driver.switchTo('hostless')
  const back = await driver.turn({
    model: [model.shell('t5', 'cat notes.md && demi host shell --host alpha "cat notes.md"'), model.say('back')],
  })
  expect(itemsText(back.requests[0]!.items)).toContain('[Execution target switched]')
  expect(itemsText(back.requests[0]!.items)).toContain('Previous target: workspace "alpha workspace"')
  expect(back.received[0]).toContain('alpha\nbeta\ngamma\nalpha\ndelta\ngamma')
  expect(driver.lastText()).toBe('back')
}, 60_000)

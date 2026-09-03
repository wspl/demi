import { readFile } from 'node:fs/promises'
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { World } from './world'
import { expected, model, type Target } from './driver'

// S1 — the file workflow: create through a heredoc, read, edit, list, across
// four turns; the text the model receives equals the file's content, and the
// file is where the target keeps it.

let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'] })
})

afterAll(async () => {
  await world.close()
})

describe.each<Target>(['hostless', 'runner:alpha'])('S1 file workflow on %s', (target) => {
  test('create, read, edit, list', async () => {
    const driver = await world.conversation(target)
    const created = await driver.turn({
      model: [model.shell('t1', "mkdir src && cd src && demi file create notes.md <<'EOF'\nalpha\nbeta\ngamma\nEOF"), model.say('created')],
    })
    expect(created.received[0]).toContain('exitCode: 0')
    expect(created.received[0]).toContain('Created notes.md')
    expect(await driver.readFile('src/notes.md')).toBe('alpha\nbeta\ngamma\n')

    // The default shell keeps its cwd between turns.
    const read = await driver.turn({
      model: [model.shell('t2', 'pwd && demi file read notes.md | grep -n a | sort -r'), model.say('read')],
    })
    expect(read.received[0]).toContain('/src\n3:gamma\n2:beta\n1:alpha')

    const edited = await driver.turn({
      model: [model.shell('t3', 'demi file edit notes.md --old beta --new delta && cat notes.md'), model.say('edited')],
    })
    expect(edited.received[0]).toContain('Edited notes.md\nalpha\ndelta\ngamma')
    expect(await driver.readFile('src/notes.md')).toBe('alpha\ndelta\ngamma\n')

    const listed = await driver.turn({
      model: [model.shell('t4', 'ls && demi host current'), model.say('listed')],
    })
    expect(listed.received[0]).toContain('notes.md')
    expect(listed.received[0]).toContain(expected(target).hostCurrent)
    expect(driver.lastText()).toBe('listed')
  }, 30_000)
})

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { World } from './world'
import { itemsText } from './model'
import { model, type Target } from './driver'

// S5 — subagents: `demi agent spawn` with the `explore` profile, then
// `default`; the child runs commands on the same target; the parent reads
// the result. Parent and child share the target; a profile is a prompt, not
// a restriction; the parent's transcript carries the subagent frames.

let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'] })
})

afterAll(async () => {
  await world.close()
})

describe.each<Target>(['hostless', 'runner:alpha'])('S5 subagents on %s', (target) => {
  test('an explore child works on the parent\'s files like any child; a default child writes', async () => {
    const driver = await world.conversation(target)
    await driver.turn({
      model: [model.shell('t1', "demi file create notes.md <<'EOF'\nthe answer is 42\nEOF"), model.say('written')],
    })

    // The explore child: sees only its brief and works on the same target as its parent.
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
      model: [model.shell('t2', "demi agent spawn 'Read notes.md and report its content' --profile explore --description reader", 10_000), model.say('explored')],
    })
    expect(brief).toContain('Read notes.md and report its content')
    expect(brief).not.toContain('written')
    const childRequests = world.model.requests.filter((request) => request.sessionId !== driver.id)
    const childSaw = itemsText(childRequests.at(-1)!.items)
    expect(childSaw).toContain('the answer is 42')
    expect(childSaw).toContain('Created blocked.md')
    expect(explored.received[0]).toContain('subagentId:')
    expect(explored.received[0]).toContain('the file says 42; I wrote too')
    expect(await driver.readFile('blocked.md')).toBe('nope\n')

    // The default child writes where the parent then reads.
    world.model.scriptChild(model.shell('c3', "demi file create reply.md <<'EOF'\nfrom the child\nEOF"), model.say('wrote reply.md'))
    const delegated = await driver.turn({
      model: [
        model.shell('t3', "demi agent spawn 'Create reply.md' --description writer", 10_000),
        model.shell('t4', 'cat reply.md'),
        model.say('delegated'),
      ],
    })
    expect(delegated.received[0]).toContain('wrote reply.md')
    expect(delegated.received[1]).toContain('from the child')

    // The parent's stream carried the subagent lifecycle for both children.
    const lifecycle = driver.events.filter((event) => event.type === 'subagent').map((event) => (event.type === 'subagent' ? event.event : ''))
    expect(lifecycle).toEqual(['started', 'closed', 'started', 'closed'])
  }, 60_000)
})


test('a cross-host command preserves the child node storage scope', async () => {
  const driver = await world.conversation('runner:alpha')
  await driver.turn({ model: [model.shell('scope-root', 'demi todo add root-only'), model.say('ready')] })
  world.model.scriptChild(
    model.shell('scope-remote', 'demi host shell --host alpha "demi todo add child-only"'),
    model.shell('scope-child-list', 'demi todo list'), model.say('child done'),
  )
  const parent = await driver.turn({ model: [model.shell('scope-spawn', "demi agent spawn 'add a todo on alpha'"), model.shell('scope-root-list', 'demi todo list'), model.say('done')] })
  expect(parent.received.at(-1)).toContain('root-only')
  expect(parent.received.at(-1)).not.toContain('child-only')
  const child = world.model.requests.filter(request => request.sessionId !== driver.id && request.items.some(item => item.type === 'tool_result' && item.toolUseId === 'scope-child-list')).at(-1)
  expect(itemsText(child!.items)).toContain('child-only')
})

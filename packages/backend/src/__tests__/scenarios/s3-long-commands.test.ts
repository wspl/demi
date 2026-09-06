import { existsSync } from 'node:fs'
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import type { InferenceRequest } from '@demicodes/provider'
import { delay, waitFor } from '@demicodes/utils'
import { FakeProvisioner } from './fake-provisioner'
import { World } from './world'
import { model, type Target, type TurnScript } from './driver'

// S3 — long commands and steering: a command outliving its window polled
// with shell_status to its end, stdin fed with shell_write, a second command
// stopped with shell_abort, a background job on a runner. The status machine
// as the model sees it; nothing left running after the abort.

const fake = new FakeProvisioner()
let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'], managedHosts: { provisioner: fake, config: { hostsPerUser: 30 } } })
})

afterAll(async () => {
  await world.close()
  await fake.close()
})

/** The last tool result the model was shown. */
function lastResult(request: InferenceRequest): string {
  const last = request.items.at(-1)
  return last?.type === 'tool_result' ? last.output.map((b) => (b.type === 'text' ? b.text : '')).join('\n') : ''
}

/** The command handle in the last tool result the model was shown. */
function handleIn(request: InferenceRequest): string {
  const text = lastResult(request)
  const commandId = /commandId: (\S+)/.exec(text)?.[1]
  if (!commandId) throw new Error(`no command handle in: ${text}`)
  return commandId
}

/** Polls the running command with shell_status every `everyMs` until it exits, then says `text`. */
function pollThenSay(driverId: string, text: string, everyMs = 150): TurnScript {
  let polls = 0
  const poll: TurnScript = (request: InferenceRequest) => {
    if (!lastResult(request).includes('status: running')) return model.say(text)
    const commandId = handleIn(request)
    world.model.script(driverId, poll)
    return (async function* () {
      await delay(everyMs)
      yield* model.tool(`poll-${(polls += 1)}`, 'shell_status', { commandId })
    })()
  }
  return poll
}

describe.each<Target>(['hostless', 'runner:alpha'])('S3 long commands on %s', (target) => {
  test('a command polled to its end with shell_status', async () => {
    const driver = await world.conversation(target)
    world.model.scriptChild(model.slowSay('child done', 1_200))
    const turn = await driver.turn({
      model: [model.shell('t1', "demi agent spawn 'take a while' --description slow", 200), pollThenSay(driver.id, 'finished')],
    })
    expect(turn.received[0]).toContain('status: running')
    expect(turn.received.length).toBeGreaterThan(2)
    for (const middle of turn.received.slice(1, -1)) expect(middle).toContain('status: running')
    const last = turn.received.at(-1)!
    expect(last).toContain('status: exited')
    expect(last).toContain('exitCode: 0')
    // Each view carries the streams' deltas since the previous one: the child's
    // result arrives in whichever poll follows the command's write, the exit in
    // the same view or the next.
    expect(turn.received.join('\n')).toContain('child done')
    expect(driver.lastText()).toBe('finished')
  }, 30_000)

  test('stdin fed with shell_write reaches the command', async () => {
    // As under bash, a command whose stdin is not redirected reads the
    // script's own: `head -n 1` waits for the line shell_write feeds.
    const driver = await world.conversation(target)
    const turn = await driver.turn({
      model: [
        model.shell('t1', 'echo -n "got: " && head -n 1 | tr a-z A-Z', 200),
        (request) => model.tool('t2', 'shell_write', { commandId: handleIn(request), stdin: 'hello there\n' }),
        pollThenSay(driver.id, 'fed'),
      ],
    })
    expect(turn.received[0]).toContain('status: running')
    expect(turn.received[0]).toContain('got: ')
    const last = turn.received.at(-1)!
    expect(last).toContain('HELLO THERE')
    expect(last).toContain('status: exited')
  }, 30_000)

  test('a second command stopped with shell_abort while another already ended', async () => {
    const driver = await world.conversation(target)
    world.model.scriptChild(model.slowSay('never read', 5_000))
    const turn = await driver.turn({
      model: [
        model.shell('t1', 'echo first'),
        model.shell('t2', "demi agent spawn 'hang around' --description hang", 200),
        (request) => model.tool('t3', 'shell_abort', { commandId: handleIn(request) }),
        model.shell('t4', 'echo after'),
        model.say('stopped'),
      ],
    })
    expect(turn.received[0]).toContain('first')
    expect(turn.received[1]).toContain('status: running')
    expect(turn.received[2]).toContain('status: aborted')
    expect(turn.received[3]).toContain('after')
  }, 30_000)
})

describe('S3 on a runner only', () => {
  test('a background job outlives its command', async () => {
    const driver = await world.conversation('runner:alpha')
    const turn = await driver.turn({
      model: [model.shell('t1', '(sleep 0.5; echo done > bg.txt) > /dev/null 2>&1 & echo started'), model.say('backgrounded')],
    })
    expect(turn.received[0]).toContain('started')
    expect(turn.received[0]).toContain('status: exited')
    await waitFor(() => existsSync(driver.filePath('bg.txt')), undefined, { timeoutMs: 5_000 })
    const next = await driver.turn({ model: [model.shell('t2', 'cat bg.txt'), model.say('seen')] })
    expect(next.received[0]).toContain('done')
  }, 30_000)
})

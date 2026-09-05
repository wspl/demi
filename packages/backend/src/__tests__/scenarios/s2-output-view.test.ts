import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { JOB_VIEW_BYTES } from '@demicodes/runner-protocol'
import { World } from './world'
import { expected, model, type Target } from './driver'

// S2 — the output view: a command printing far past the view budget, a
// binary final stream, a non-zero exit, a command hitting its observation
// window. What the model receives follows the boundary rules; on a runner
// the wire bytes are the view (the teardown audit).

let world: World

beforeAll(async () => {
  world = await World.create({ runners: ['alpha'] })
})

afterAll(async () => {
  await world.close()
})

/** Text past the runner's head+tail view and the model's preview, within the hostless capture limit. */
const BIG_LINES = Array.from({ length: 4_000 }, (_, i) => `line ${String(i).padStart(5, '0')} ${'x'.repeat(20)}`)
const BIG_TEXT = `${BIG_LINES.join('\n')}\n`
const PNG_BYTES = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01])

describe.each<Target>(['hostless', 'runner:alpha'])('S2 output view on %s', (target) => {
  test('a stream past the view budget', async () => {
    const driver = await world.conversation(target)
    await driver.upload('big.txt', new TextEncoder().encode(BIG_TEXT))
    expect(BIG_TEXT.length).toBeGreaterThan(3 * JOB_VIEW_BYTES)

    const turn = await driver.turn({ model: [model.shell('t1', 'cat big.txt'), model.say('seen')] })
    const received = turn.received[0]!
    expect(received).toContain('exitCode: 0')
    expect(received).toContain(`stdoutBytes: ${BIG_TEXT.length}`)
    // The preview is the head within the model's budget, with the rule for the rest.
    expect(received).toContain('line 00000')
    expect(received).not.toContain('line 03999')
    expect(received).toContain(expected(target).previewTruncated)
    // The shell view: the runner shows the head, a gap note and the tail; hostless shows the capture.
    const status = turn.shell.find((event) => event.status.status === 'exited')?.status
    const text = status?.status === 'exited' ? status.stdout.delta : ''
    expect(text).toContain('line 03999')
    const gap = expected(target).gapNote
    if (gap) expect(text).toContain(gap)
    else expect(text).toBe(BIG_TEXT)
  }, 30_000)

  test('a binary final stream', async () => {
    const driver = await world.conversation(target)
    await driver.upload('image.png', PNG_BYTES)
    const turn = await driver.turn({ model: [model.shell('t1', 'cat image.png'), model.say('seen')] })
    const received = turn.received[0]!
    expect(received).toContain('exitCode: 0')
    expect(received).toContain(`<binary stdout: ${PNG_BYTES.length} bytes${expected(target).binaryPlaceholder}`)
    // The model does not accept png here, so nothing is attached and the note says where the bytes are.
    expect(received).toContain('Binary stdout is image/png, which this model does not accept natively')
    expect(received).toContain(expected(target).binaryKept)
  }, 30_000)

  test('a non-zero exit with stderr', async () => {
    const driver = await world.conversation(target)
    const turn = await driver.turn({ model: [model.shell('t1', 'cat nope.txt; echo after'), model.say('seen')] })
    const received = turn.received[0]!
    expect(received).toContain('exitCode: 0')
    expect(received).toContain('nope.txt')
    expect(received).toContain('after')
    const failed = await driver.turn({ model: [model.shell('t2', 'cat nope.txt'), model.say('seen')] })
    expect(failed.received[0]).toContain('exitCode: 1')
    expect(failed.received[0]).toContain('No such file or directory')
  }, 30_000)

  test('a command outliving its observation window', async () => {
    const driver = await world.conversation(target)
    // A subagent whose model takes longer than the window: `demi agent spawn`
    // is the long-running command both targets share.
    world.model.scriptChild(model.slowSay('child done', 3_000))
    const turn = await driver.turn({
      model: [
        model.shell('t1', "demi agent spawn 'take a while' --description slow", 300),
        (request) => {
          const last = request.items.at(-1)
          const text = last?.type === 'tool_result' ? last.output.map((b) => (b.type === 'text' ? b.text : '')).join('\n') : ''
          const commandId = /commandId: (\S+)/.exec(text)?.[1]
          if (!commandId) throw new Error(`no command handle in: ${text}`)
          return model.tool('t2', 'shell_abort', { commandId })
        },
        model.say('stopped'),
      ],
    })
    expect(turn.received[0]).toContain('status: running')
    expect(turn.received[0]).toContain('next: the child agent is still working')
    expect(turn.received[0]).toContain('Do not poll with shell_status or timed yields')
    expect(turn.received[0]).not.toContain('next: command is still running')
    expect(turn.received[1]).toContain('status: aborted')
    expect(turn.received[1]).toContain('next: command was intentionally stopped.')
  }, 30_000)
})

import { expect, test } from 'bun:test'
import { World } from './world'
import { model } from './driver'

// Real runner HTTP transfers and RPC, with a scripted provider only.
test('a cross-host RPC stays connected before its first byte and between bytes past the default HTTP idle timeout', async () => {
  const world = await World.create({ runners: ['alpha'] })
  try {
    const driver = await world.conversation('runner:alpha')
    const turn = await driver.turn({ model: [
      model.shell(
        'quiet-rpc',
        'demi host shell --host alpha "sleep 14; echo first; sleep 14; echo last"',
        35_000,
      ),
      model.say('finished'),
    ] })
    expect(turn.received[0]).toContain('exitCode: 0')
    expect(turn.received[0]).toContain('first\nlast')
    const transfers = world.frames.filter((frame) =>
      frame.direction === 'in' && frame.message.type === 'pipe_done'
    )
    expect(transfers.length).toBeGreaterThan(0)
    for (const frame of transfers) {
      if (frame.message.type === 'pipe_done')
        expect(frame.message.ok).toBe(true)
    }
  } finally {
    await world.close()
  }
}, 45_000)

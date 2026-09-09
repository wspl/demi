import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { AgentClient } from '../client/client'
import type { ClientFrame, ServerFrame } from '../protocol/frames'

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'stub',
    name: 'Stub',
    contextWindow: 1000,
    inputLimit: null,
    outputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}
function harness() {
  const sent: ClientFrame[] = []
  let receive: (frame: ServerFrame) => void = () => {}
  let closes = 0
  const client = new AgentClient({
    send: (frame) => {
      sent.push(frame)
    },
    onFrame(handler) {
      receive = handler
      return () => {
        receive = () => {}
      }
    },
    close: () => {
      closes += 1
    },
  })
  return {
    client,
    sent,
    receive: (frame: ServerFrame) => receive(frame),
    closes: () => closes,
  }
}

test('submit confirms its own transcript entry before the model finishes', async () => {
  const h = harness()
  let accepted = false
  const pending = h.client
    .submit([
      {
        type: 'text',
        text: 'hello',
      },
    ])
    .then(() => {
      accepted = true
    })
  const frame = h.sent[0]!
  if (frame.type !== 'send') {
    throw new Error('Expected send')
  }
  h.receive({
    type: 'phase',
    phase: 'running',
  })
  await Promise.resolve()
  expect(accepted).toBe(false)
  h.receive({
    type: 'transcript_reset',
    revision: 1,
    blocks: [
      {
        type: 'user',
        id: 'user-block',
        turnId: frame.messageId,
        createdAt: '2026-09-10T00:00:00Z',
        model,
        content: frame.content,
        preamble: null,
      },
    ],
  })
  await pending
  expect(accepted).toBe(true)
  h.client.disconnect()
  expect(h.sent.map((frame) => frame.type)).toEqual(['send'])
  expect(h.closes()).toBe(1)
})

test('submit confirms queue admission while a different turn is running', async () => {
  const h = harness()
  const pending = h.client.submit([
    {
      type: 'text',
      text: 'next',
    },
  ])
  const frame = h.sent[0]!
  if (frame.type !== 'send') {
    throw new Error('Expected send')
  }
  h.receive({
    type: 'queue',
    queue: [
      {
        id: frame.messageId,
        text: 'next',
        content: frame.content,
      },
    ],
  })
  await pending
  h.client.disconnect()
})

for (const outcome of ['rejected', 'closed', 'disconnected'] as const) {
  test(`submit rejects when ${outcome} happens before confirmation`, async () => {
    const h = harness()
    const pending = h.client.submit([
      {
        type: 'text',
        text: 'keep draft',
      },
    ])
    const result = pending.catch((error) => error)
    if (outcome === 'disconnected') {
      h.client.disconnect()
    } else if (outcome === 'rejected') {
      h.receive({
        type: 'rejected',
        command: 'send',
        reason: 'Archived',
      })
    } else {
      h.receive({ type: 'closed' })
    }
    expect(await result).toBeInstanceOf(Error)
    h.client.disconnect()
  })
}

test('submitting on an already detached client fails immediately', async () => {
  const h = harness()
  h.client.disconnect()
  await expect(
    h.client.submit([
      {
        type: 'text',
        text: 'retained',
      },
    ]),
  ).rejects.toThrow('closed')
  expect(h.sent).toHaveLength(0)
})

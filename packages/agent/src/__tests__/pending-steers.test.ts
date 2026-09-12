import { expect, test } from 'bun:test'
import type { Block, ModelSelection, PendingSteer } from '@demicodes/core'
import { AgentClient } from '../client/client'
import type { ServerFrame } from '../protocol/frames'
import { PendingSteerQueue } from '../session/steer-queue'

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'stub',
    name: 'Stub',
    contextWindow: 100_000,
    outputLimit: null,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: []
  },
  thinking: null,
}

function pending(id: string): PendingSteer {
  return {
    id,
    turnId: 'turn',
    model,
    content: [{ type: 'text', text: 'identical guidance' }]
  }
}

function clientHarness() {
  let receive: (frame: ServerFrame) => void = () => {}
  const client = new AgentClient({
    send() {},
    close() {},
    onFrame(handler) {
      receive = handler
      return () => {
        receive = () => {}
      }
    },
  })
  return { client, receive: (frame: ServerFrame) => receive(frame) }
}

test(
  'pending snapshots hide internal wakeups without removing them from the execution queue',
  () => {
    const queue = new PendingSteerQueue()
    queue.add({ ...pending('hidden'), hidden: true })
    queue.add(pending('user'))
    expect(queue.snapshot()).toEqual([pending('user')])
    expect(queue.snapshot()[0]).not.toHaveProperty('hidden')
    const snapshot = queue.snapshot()
    snapshot[0]!.content.length = 0
    snapshot[0]!.model.model.id = 'caller edit'
    expect(queue.snapshot()).toEqual([pending('user')])
    expect(queue.takeForTurn('turn').map((steer) => steer.agentMessage ? steer.agentMessage.id : steer.id)).toEqual([
      'hidden',
      'user'
    ])
    expect(queue.snapshot()).toEqual([])
  }
)

test(
  'AgentClient removes pending steers by id, including when history arrives before the list',
  () => {
    const { client, receive } = clientHarness()
    const history: Block = {
      ...pending('first'),
      type: 'steer',
      createdAt: '2026-09-08T00:00:00Z'
    }
    receive({ type: 'transcript_reset', epoch: 'test-epoch', blocks: [history], revision: 1 })
    receive({
      type: 'pending_steers',
      pendingSteers: [pending('first'), pending('second')]
    })
    expect(client.pendingSteers()).toEqual([pending('second')])
    // The two messages have identical content. Only the id determines which entered history.
    receive({
      type: 'transcript_patch',
      patches: [{
        op: 'add',
        path: ['blocks', 1],
        value: { ...history, id: 'second' }
      }],
      revision: 2
    })
    expect(client.pendingSteers()).toEqual([])
    receive({ type: 'pending_steers', pendingSteers: [] })
    receive({ type: 'closed' })
    expect(client.pendingSteers()).toEqual([])
  }
)

test(
  'AgentClient validates pending data and owns copies of binary attachments',
  () => {
    const { client, receive } = clientHarness()
    const steer: PendingSteer = {
      ...pending('binary'),
      content: [{
        type: 'image',
        source: {
          type: 'binary',
          mediaType: 'image/png',
          data: new Uint8Array([1, 2, 3])
        }
      }],
    }
    receive({ type: 'pending_steers', pendingSteers: [steer] })
    const snapshot = client.pendingSteers()
    const image = snapshot[0]!.content[0]!
    if (image.type !== 'image' || image.source.type !== 'binary')
      throw new Error('Expected binary image')
    image.source.data[0] = 99
    steer.content.length = 0
    expect(client.pendingSteers()[0]!.content).toEqual([
      {
        type: 'image',
        source: {
          type: 'binary',
          mediaType: 'image/png',
          data: new Uint8Array([1, 2, 3])
        }
      },
    ])
    expect(
      () => receive(
        { type: 'pending_steers', pendingSteers: [{ id: 7 }] } as unknown as ServerFrame
      )
    ).toThrow()
    expect(client.pendingSteers()[0]!.id).toBe('binary')
    receive({ type: 'closed' })
    expect(client.pendingSteers()).toEqual([])
  }
)

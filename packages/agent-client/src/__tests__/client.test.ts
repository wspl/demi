import { expect, test } from 'bun:test'
import type { ClientFrame } from '@demicodes/protocol'
import { EditRejectedError, SessionError } from '../client'
import { createdAt, harness, model, pendingSteer, settle, text, user } from './harness'

function sentOfType<Type extends ClientFrame['type']>(sent: ClientFrame[], type: Type): Extract<ClientFrame, { type: Type }> {
  const frame = sent.find((candidate): candidate is Extract<ClientFrame, { type: Type }> => candidate.type === type)
  if (!frame) {
    throw new Error(`no ${type} frame was sent`)
  }
  return frame
}

test('open resolves on opened and sends only the model', async () => {
  const h = harness()
  const opening = h.client.open(model)
  expect(h.sent).toEqual([{ type: 'open', model }])
  h.receive({ type: 'opened' })
  await opening
})

test('a submitted message is confirmed by its own user block or queue entry, before its turn ends', async () => {
  const h = harness()
  let confirmed = false
  const pending = h.client.submit([{ type: 'text', text: 'hello' }]).then(() => {
    confirmed = true
  })
  const send = sentOfType(h.sent, 'send')
  h.receive({ type: 'phase', phase: 'running' })
  h.receive({ type: 'transcript_reset', blocks: [user('u0', 'another-turn', 'hello')], version: { epoch: 'e', revision: 1 } })
  await settle()
  expect(confirmed).toBe(false)
  h.receive({ type: 'transcript_patch', patches: [{ op: 'add', index: 1, value: user('u1', send.messageId, 'hello') }], revision: 2 })
  await pending

  const queued = h.client.submit([{ type: 'text', text: 'next' }], 'chosen-id')
  expect(h.sent.at(-1)).toMatchObject({ type: 'send', messageId: 'chosen-id' })
  h.receive({ type: 'queue', queue: [{ id: 'chosen-id', content: [{ type: 'text', text: 'next' }] }] })
  await queued
})

for (const outcome of ['rejected', 'error', 'closed', 'disconnected'] as const) {
  test(`a submission fails when the connection answers ${outcome} before confirming it`, async () => {
    const h = harness()
    const result = h.client.submit([{ type: 'text', text: 'keep the draft' }]).catch((error: unknown) => error)
    if (outcome === 'rejected') {
      h.receive({ type: 'rejected', command: 'send', reason: 'The conversation is archived' })
    } else if (outcome === 'error') {
      h.receive({ type: 'error', message: 'invalid frame', code: 'invalid_frame' })
    } else if (outcome === 'closed') {
      h.receive({ type: 'closed' })
    } else {
      h.client.disconnect()
    }
    expect(await result).toBeInstanceOf(Error)
  })
}

test('a detached client fails every wait at once and sends nothing', async () => {
  const h = harness()
  h.client.disconnect()
  await expect(h.client.submit([{ type: 'text', text: 'kept' }])).rejects.toThrow('closed')
  await expect(h.client.steer([{ type: 'text', text: 'kept' }])).rejects.toThrow('closed')
  await expect(h.client.abort()).rejects.toThrow('closed')
  h.client.cancelPendingSteer('s1')
  expect(h.sent).toEqual([])
  expect(h.closes()).toBe(1)
})

test('an edit waits for its own receipt, even after the replacement appears', async () => {
  const h = harness()
  const request = {
    operationId: 'edit-1',
    targetBlockId: 'u1',
    version: { epoch: 'e', revision: 1 },
    content: [{ type: 'text' as const, text: 'edited' }],
  }
  let accepted = false
  const pending = h.client.editAndSend(request).then(() => {
    accepted = true
  })
  expect(h.sent).toEqual([{ type: 'edit_and_send', request }])
  h.receive({ type: 'transcript_reset', blocks: [user('u2', 'turn-2', 'edited')], version: { epoch: 'e', revision: 2 } })
  h.receive({ type: 'edit_result', operationId: 'edit-0', outcome: { status: 'accepted', turnId: 'turn-0' } })
  await settle()
  expect(accepted).toBe(false)
  h.receive({ type: 'edit_result', operationId: 'edit-1', outcome: { status: 'accepted', turnId: 'turn-2' } })
  await pending
})

for (const ending of ['rejected', 'disconnected'] as const) {
  test(`an edit that ends ${ending} tells a safe correction from an uncertain outcome`, async () => {
    const h = harness()
    const request = {
      operationId: 'edit-1',
      targetBlockId: 'u1',
      version: { epoch: 'e', revision: 1 },
      content: [{ type: 'text' as const, text: 'edited' }],
    }
    const result = h.client.editAndSend(request).catch((error: unknown) => error)
    if (ending === 'rejected') {
      h.receive({ type: 'edit_result', operationId: 'edit-1', outcome: { status: 'rejected', reason: 'stale' } })
    } else {
      h.client.disconnect()
    }
    const error = await result
    expect(error).toBeInstanceOf(Error)
    expect(error instanceof EditRejectedError).toBe(ending === 'rejected')
  })
}

test('a queued send ends when the next message runs or the session is idle, and one leaving the queue ends without running', async () => {
  const h = harness()
  const settled: string[] = []
  const first = h.client.send([{ type: 'text', text: 'first' }]).then(() => settled.push('first'))
  const second = h.client.send([{ type: 'text', text: 'second' }]).then(() => settled.push('second'))
  const third = h.client.send([{ type: 'text', text: 'third' }]).then(() => settled.push('third'))
  const [firstSend, secondSend, thirdSend] = h.sent.filter((frame) => frame.type === 'send')
  if (firstSend?.type !== 'send' || secondSend?.type !== 'send' || thirdSend?.type !== 'send') {
    throw new Error('three sends were expected')
  }
  h.receive({ type: 'phase', phase: 'running' })
  h.receive({ type: 'transcript_reset', blocks: [user('u1', firstSend.messageId, 'first')], version: { epoch: 'e', revision: 1 } })
  h.receive({ type: 'queue', queue: [{ id: secondSend.messageId, content: [] }, { id: thirdSend.messageId, content: [] }] })
  h.client.dequeueMessage(secondSend.messageId)
  await second
  expect(settled).toEqual(['second'])
  // The third message runs right after the first, with no idle between them.
  h.receive({ type: 'queue', queue: [] })
  h.receive({ type: 'transcript_patch', patches: [{ op: 'add', index: 1, value: user('u3', thirdSend.messageId, 'third') }], revision: 2 })
  await first
  expect(settled).toEqual(['second', 'first'])
  h.receive({ type: 'phase', phase: 'idle' })
  await third
  expect(settled).toEqual(['second', 'first', 'third'])
})

test('a queued message turned into a steer settles its send when the steer is accepted', async () => {
  const h = harness()
  const queued = h.client.send([{ type: 'text', text: 'queued' }])
  const send = sentOfType(h.sent, 'send')
  const steering = h.client.steerQueuedMessage(send.messageId, 'steer-1')
  expect(h.sent.at(-1)).toEqual({ type: 'steer_queued_message', messageId: send.messageId, steerId: 'steer-1' })
  h.receive({ type: 'steer_result', steerId: 'steer-1', outcome: { status: 'accepted' } })
  await steering
  await queued
})

test('a steer resolves on its own accepted result and fails on a rejected one', async () => {
  const h = harness()
  const accepted = h.client.steer([{ type: 'text', text: 'also this' }], 'steer-1')
  const rejected = h.client.steer([{ type: 'text', text: 'and this' }], 'steer-2').catch((error: unknown) => error)
  h.receive({ type: 'steer_result', steerId: 'steer-2', outcome: { status: 'rejected', reason: 'No session is open on this connection' } })
  h.receive({ type: 'steer_result', steerId: 'steer-1', outcome: { status: 'accepted' } })
  await accepted
  expect(await rejected).toMatchObject({ message: 'No session is open on this connection' })
})

test('the pending steers leave by id, even when the history arrives before the list', () => {
  const h = harness()
  const written = { ...pendingSteer('first'), type: 'steer' as const, createdAt }
  h.receive({ type: 'transcript_reset', blocks: [written], version: { epoch: 'e', revision: 1 } })
  h.receive({ type: 'pending_steers', pendingSteers: [pendingSteer('first'), pendingSteer('second')] })
  expect(h.client.pendingSteers()).toEqual([pendingSteer('second')])
  // Both have the same content; only the id says which one the transcript holds.
  h.receive({ type: 'transcript_patch', patches: [{ op: 'add', index: 1, value: { ...written, id: 'second' } }], revision: 2 })
  expect(h.client.pendingSteers()).toEqual([])
  expect(h.events.filter((event) => event.type === 'pending_steers').at(-1)).toEqual({ type: 'pending_steers', pendingSteers: [] })
})

test('a cancelled steer leaves with the list the server answers', () => {
  const h = harness()
  h.receive({ type: 'pending_steers', pendingSteers: [pendingSteer('first'), pendingSteer('second')] })
  h.client.cancelPendingSteer('first')
  expect(h.sent).toEqual([{ type: 'cancel_pending_steer', steerId: 'first' }])
  h.receive({ type: 'pending_steers', pendingSteers: [pendingSteer('second')] })
  expect(h.client.pendingSteers()).toEqual([pendingSteer('second')])
})

test('the pending steers the client answers are its own copies', () => {
  const h = harness()
  h.receive({ type: 'pending_steers', pendingSteers: [pendingSteer('first')] })
  const copy = h.client.pendingSteers()
  copy[0]!.content.length = 0
  copy.length = 0
  expect(h.client.pendingSteers()).toEqual([pendingSteer('first')])
})

test('a frame the contract does not allow disconnects the client before anything acts on it', () => {
  for (const frame of [
    { type: 'pending_steers', pendingSteers: [{ id: 7 }] },
    { type: 'transcript_reset', blocks: [{ type: 'text' }], version: { epoch: 'e', revision: 0 } },
    { type: 'transcript_patch', patches: [{ op: 'add', path: ['blocks', 0], value: text('a', 'x') }], revision: 1 },
    { type: 'tool_progress', toolUseId: 't1', output: [] },
  ]) {
    const h = harness()
    h.receiveValue(frame)
    expect(h.events.map((event) => event.type)).toEqual(['disconnected'])
    const [event] = h.events
    expect(event?.type === 'disconnected' && event.error.message).toStartWith('Invalid server frame')
    expect(h.client.transcript().blocks).toEqual([])
    expect(h.client.pendingSteers()).toEqual([])
    expect(h.closes()).toBe(1)
  }
})

test('a patch at or before the client revision is ignored, and a gap asks for a fresh transcript', () => {
  const h = harness()
  h.receive({ type: 'transcript_reset', blocks: [text('a', 'one')], version: { epoch: 'e', revision: 1 } })
  expect(h.client.transcriptVersion()).toEqual({ epoch: 'e', revision: 1 })
  h.receive({ type: 'transcript_patch', patches: [{ op: 'append_text', index: 0, delta: ' again' }], revision: 1 })
  expect(h.client.transcript().blocks).toEqual([text('a', 'one')])

  h.receive({ type: 'transcript_patch', patches: [{ op: 'add', index: 1, value: text('c', 'three') }], revision: 3 })
  expect(h.sent).toEqual([{ type: 'sync_transcript' }])
  expect(h.client.transcriptVersion()).toBeNull()
  h.receive({ type: 'transcript_patch', patches: [{ op: 'add', index: 1, value: text('d', 'four') }], revision: 4 })
  expect(h.sent).toEqual([{ type: 'sync_transcript' }])
  expect(h.client.transcript().blocks).toEqual([text('a', 'one')])

  h.receive({
    type: 'transcript_reset',
    blocks: [text('a', 'one'), text('b', 'two'), text('c', 'three')],
    version: { epoch: 'e', revision: 4 },
  })
  h.receive({ type: 'transcript_patch', patches: [{ op: 'append_text', index: 2, delta: ' more' }], revision: 5 })
  expect(h.client.transcript().blocks.at(-1)).toEqual(text('c', 'three more'))
  expect(h.client.transcriptVersion()).toEqual({ epoch: 'e', revision: 5 })
})

test('each subagent transcript follows the same revision rule', () => {
  const h = harness()
  h.receive({ type: 'transcript_reset', blocks: [], version: { epoch: 'e', revision: 0 } })
  h.receive({ type: 'subagent_transcript_reset', subagentId: 'child', blocks: [], revision: 3 })
  h.receive({ type: 'subagent_transcript_patch', subagentId: 'child', patches: [{ op: 'add', index: 0, value: text('x', 'x') }], revision: 4 })
  h.receive({ type: 'subagent_transcript_patch', subagentId: 'child', patches: [{ op: 'remove', index: 0 }], revision: 4 })
  const patches = h.events.filter((event) => event.type === 'subagent_transcript_patch')
  expect(patches).toEqual([
    { type: 'subagent_transcript_patch', subagentId: 'child', patches: [{ op: 'add', index: 0, value: text('x', 'x') }], failures: {} },
  ])
  expect(h.sent).toEqual([])
  h.receive({ type: 'subagent_transcript_patch', subagentId: 'child', patches: [], revision: 6 })
  h.receive({ type: 'subagent_transcript_patch', subagentId: 'unknown', patches: [], revision: 1 })
  expect(h.sent).toEqual([{ type: 'sync_transcript' }])
})

test('closed clears the transcript, the version and the pending steers, and ends the actions', async () => {
  const h = harness()
  h.receive({ type: 'transcript_reset', blocks: [text('a', 'one')], version: { epoch: 'e', revision: 1 } })
  h.receive({ type: 'pending_steers', pendingSteers: [pendingSteer('first')] })
  const action = h.client.send([{ type: 'text', text: 'hi' }])
  const steer = h.client.steer([{ type: 'text', text: 'also' }]).catch((error: unknown) => error)
  h.receive({ type: 'closed' })
  await action
  expect(await steer).toMatchObject({ message: 'Session closed' })
  expect(h.client.transcript().blocks).toEqual([])
  expect(h.client.transcriptVersion()).toBeNull()
  expect(h.client.pendingSteers()).toEqual([])
})

test('retry, resume and compact resolve when their action ends, and a busy session refuses them', async () => {
  const h = harness()
  const retry = h.client.retry()
  h.receive({ type: 'phase', phase: 'running' })
  h.receive({ type: 'phase', phase: 'idle' })
  await retry
  const resume = h.client.resume()
  const compact = h.client.compact().catch((error: unknown) => error)
  h.receive({ type: 'phase', phase: 'running' })
  h.receive({ type: 'rejected', command: 'compact', reason: 'Session is busy (running)' })
  expect(await compact).toMatchObject({ message: 'Session is busy (running)' })
  h.receive({ type: 'phase', phase: 'idle' })
  await resume
  expect(h.sent).toEqual([{ type: 'retry' }, { type: 'resume' }, { type: 'compact' }])
})

test('a failed turn fails its action with the error frame, and the aborts answer in request order', async () => {
  const h = harness()
  const action = h.client.send([{ type: 'text', text: 'hi' }]).catch((error: unknown) => error)
  h.receive({ type: 'phase', phase: 'running' })
  const first = h.client.abort()
  const second = h.client.abort()
  h.receive({ type: 'abort_result', result: { target: 'active_provider_stream', canAbortAgain: true } })
  h.receive({ type: 'abort_result', result: { target: null, canAbortAgain: false } })
  expect(await first).toEqual({ target: 'active_provider_stream', canAbortAgain: true })
  expect(await second).toEqual({ target: null, canAbortAgain: false })
  h.receive({ type: 'error', message: 'overloaded', code: 'overloaded' })
  const error = await action
  expect(error).toBeInstanceOf(SessionError)
  expect(error).toMatchObject({ code: 'overloaded', message: 'overloaded' })
})

test('a shell write resolves on its own write result', async () => {
  const h = harness()
  let written = false
  const pending = h.client.shellWrite('cmd-1', 'y\n').then(() => {
    written = true
  })
  expect(h.sent).toEqual([{ type: 'shell_write', commandId: 'cmd-1', stdin: 'y\n' }])
  h.receive({ type: 'shell_write_result', commandId: 'cmd-2' })
  await settle()
  expect(written).toBe(false)
  h.receive({ type: 'shell_write_result', commandId: 'cmd-1' })
  await pending
})

test('close disposes the tree, then detaches; a server-side end disconnects with its reason', async () => {
  const h = harness()
  const closing = h.client.close()
  expect(h.sent).toEqual([{ type: 'close' }])
  h.receive({ type: 'closed' })
  await closing
  expect(h.closes()).toBe(1)

  const other = harness()
  const opening = other.client.open(model).catch((error: unknown) => error)
  other.end(new Error('The agent socket closed (4001 lagged)'))
  expect(await opening).toBeInstanceOf(Error)
  expect(other.events.at(-1)).toMatchObject({ type: 'disconnected', error: { message: 'The agent socket closed (4001 lagged)' } })
})

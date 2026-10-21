import { expect, jest, spyOn, test } from 'bun:test'
import type { ClientFrame } from '@demicodes/protocol'
import { ActionOutlivedError, EditRejectedError, RefusedError, SavedGoneError, SessionError } from '../client'
import { ROOT, createdAt, entry, harness, pendingSteer, settle, text, user } from './harness'

function sentOfType<Type extends ClientFrame['type']>(sent: ClientFrame[], type: Type): Extract<ClientFrame, { type: Type }> {
  const frame = sent.find((candidate): candidate is Extract<ClientFrame, { type: Type }> => candidate.type === type)
  if (!frame) {
    throw new Error(`no ${type} frame was sent`)
  }
  return frame
}

test('open resolves on opened', async () => {
  const h = harness({ opened: false })
  const opening = h.client.open({ root: ROOT })
  expect(h.sent).toEqual([{ type: 'subscribe', node: ROOT }, { type: 'open' }])
  h.receive({ type: 'opened' })
  await opening
})

// A send settles on its answer's version, before its turn ends: the step that
// took it, which shows the message as the turn's starting message or in the
// queue. Another message's answer settles nothing.
test('a submitted message settles once the client holds the version its own answer names', async () => {
  const h = harness()
  let confirmed = false
  const pending = h.client.submit([{ type: 'text', text: 'hello' }]).then(() => {
    confirmed = true
  })
  const send = sentOfType(h.sent, 'send')
  h.receive({ type: 'send_result', messageId: 'another-turn', at: null })
  h.receive({ type: 'send_result', messageId: send.messageId, at: { epoch: 'e', revision: 1 } })
  await settle()
  expect(confirmed).toBe(false)
  h.phase('running')
  await pending

  const queued = h.client.submit([{ type: 'text', text: 'next' }], 'chosen-id')
  expect(h.sent.at(-1)).toMatchObject({ type: 'send', messageId: 'chosen-id' })
  h.queue([{ id: 'chosen-id', content: [{ type: 'text', text: 'next' }] }])
  h.receive({ type: 'send_result', messageId: 'chosen-id', at: { epoch: 'e', revision: 2 } })
  await queued
})

for (const outcome of ['rejected', 'error', 'closed', 'disconnected'] as const) {
  test(`a submission fails when the connection answers ${outcome} before confirming it`, async () => {
    const h = harness()
    const result = h.client.submit([{ type: 'text', text: 'keep the draft' }]).catch((error: unknown) => error)
    if (outcome === 'rejected') {
      h.receive({ type: 'rejected', command: 'send', code: 'conversation_archived', reason: 'The conversation is archived' })
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

// A send the backend received waits for the backend's word however long
// it takes, such as a slow store of the message's files, and
// fails only with the refusal, which says why. Before, the page gave up after
// its own 30 seconds. Fake timers; instant.
test('a send has no deadline of its own and fails with the backend\'s refusal and its cause', async () => {
  const h = harness()
  let settled = false
  jest.useFakeTimers()
  let sending!: Promise<unknown>
  let saving!: Promise<unknown>
  try {
    sending = h.client.submit([{ type: 'text', text: 'see the screenshot' }]).catch((error: unknown) => error)
    saving = h.client.sendSaved('saved-1', 'turn-1', 'message').catch((error: unknown) => error)
    // Any deadline the waits set fires now.
    jest.advanceTimersByTime(10 * 60_000)
  } finally {
    jest.useRealTimers()
  }
  void sending.then(() => { settled = true })
  await settle()
  expect(settled).toBe(false)

  const failed = { code: 'send_failed', reason: 'database is locked', requestId: 'req_ffb05w4w6kxpdtz0' }
  h.receive({ type: 'rejected', command: 'send', ...failed })
  const refused = await sending
  expect(refused).toBeInstanceOf(RefusedError)
  expect(refused).toMatchObject({ code: failed.code, message: failed.reason, requestId: failed.requestId })
  // A saved message's send refused for another reason is no message gone from the list.
  h.receive({ type: 'rejected', command: 'send_saved', ...failed })
  const savedRefusal = await saving
  expect(savedRefusal).toBeInstanceOf(RefusedError)
  expect(savedRefusal).not.toBeInstanceOf(SavedGoneError)
})

test('a detached client fails every wait at once and sends nothing', async () => {
  const h = harness()
  h.client.disconnect()
  await expect(h.client.submit([{ type: 'text', text: 'kept' }])).rejects.toThrow('closed')
  await expect(h.client.steer([{ type: 'text', text: 'kept' }])).rejects.toThrow('closed')
  await expect(h.client.abort()).rejects.toThrow('closed')
  await expect(h.client.cancelPendingSteer('s1')).rejects.toThrow('closed')
  expect(h.sent).toEqual([])
  expect(h.closes()).toBe(1)
})

test('a frame the socket cannot send disconnects the client, and the wait fails with the socket\'s error', async () => {
  const h = harness()
  const refusal = new DOMException('Still in CONNECTING state.', 'InvalidStateError')
  h.refuseSends(refusal)
  expect(await h.client.submit([{ type: 'text', text: 'hello' }]).catch((error: unknown) => error)).toBe(refusal)
  expect(h.events).toEqual([{ type: 'disconnected', error: refusal }])
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
  h.receive({ type: 'transcript_reset', node: ROOT, start: 0, length: 1, blocks: [user('u2', 'turn-2', 'edited')], version: { epoch: 'e', revision: 2 } })
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
      h.receive({ type: 'edit_result', operationId: 'edit-1', outcome: { status: 'rejected', code: 'edit_stale', reason: 'stale' } })
    } else {
      h.client.disconnect()
    }
    const error = await result
    expect(error).toBeInstanceOf(Error)
    expect(error instanceof EditRejectedError).toBe(ending === 'rejected')
  })
}

// A connection that ends while the session runs an action it took leaves the
// wait for the action's end, not a refusal; one the session has not answered
// yet stays uncertain, and fails with the connection's own error.
test('a connection that ends fails the wait of an action the session took apart from one it has not answered', async () => {
  const h = harness()
  const answered = h.client.resume('resume-1').catch((error: unknown) => error)
  h.phase('running')
  const unanswered = h.client.compact().catch((error: unknown) => error)
  const lost = new Error('The agent socket went silent')
  h.client.disconnect(lost)
  const outlived = await answered
  expect(outlived).toBeInstanceOf(ActionOutlivedError)
  expect((outlived as ActionOutlivedError).cause).toBe(lost)
  expect(await unanswered).toBe(lost)
})

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
  h.phase('running')
  h.receive({ type: 'transcript_reset', node: ROOT, start: 0, length: 1, blocks: [user('u1', firstSend.messageId, 'first')], version: { epoch: 'e', revision: 1 } })
  h.queue([{ id: secondSend.messageId, content: [] }, { id: thirdSend.messageId, content: [] }])
  const dequeued = h.client.dequeueMessage(secondSend.messageId)
  h.queue([{ id: thirdSend.messageId, content: [] }])
  h.receive({ type: 'remove_result', id: secondSend.messageId, outcome: 'removed', at: { epoch: 'e', revision: 3 } })
  expect(await dequeued).toBe('removed')
  await second
  expect(settled).toEqual(['second'])
  // The third message runs right after the first, with no idle between them.
  h.queue([])
  h.receive({ type: 'transcript_patch', node: ROOT, patches: [{ op: 'add', index: 1, value: user('u3', thirdSend.messageId, 'third') }], revision: 2 })
  await first
  expect(settled).toEqual(['second', 'first'])
  h.phase('idle')
  await third
  expect(settled).toEqual(['second', 'first', 'third'])
})

// The answer is no state: a steer accepted while the version that shows it is
// late stays the page's own until the client holds that version. Before, the
// steer resolved on its answer, the page dropped its row, and the message
// showed nowhere until the list came.
test('an accepted steer resolves once the client holds the version its answer names, and a rejected one fails', async () => {
  const h = harness()
  let delivered = false
  const accepted = h.client.steer([{ type: 'text', text: 'also this' }], 'steer-1').then(() => {
    delivered = true
  })
  const rejected = h.client.steer([{ type: 'text', text: 'and this' }], 'steer-2').catch((error: unknown) => error)
  h.receive({ type: 'steer_result', steerId: 'steer-2', outcome: { status: 'rejected', code: 'no_session', reason: 'No session is open on this connection' } })
  h.receive({ type: 'steer_result', steerId: 'steer-1', outcome: { status: 'accepted', at: { epoch: 'e', revision: 1 } } })
  await settle()
  expect(delivered).toBe(false)
  h.steers([pendingSteer('steer-1')])
  await accepted
  expect(await rejected).toMatchObject({ message: 'No session is open on this connection' })
})

test('a steer the root lists before its answer resolves with the answer', async () => {
  const h = harness()
  const steering = h.client.steer([{ type: 'text', text: 'also this' }], 'steer-1')
  h.steers([pendingSteer('steer-1')])
  h.receive({ type: 'steer_result', steerId: 'steer-1', outcome: { status: 'accepted', at: { epoch: 'e', revision: 1 } } })
  await steering
})

// Recall settles on the version too: a removal answered `removed` while the
// version that drops the steer is late resolves once the client holds it, so
// the page never shows a recalled steer again in between.
test('a removal resolves once the client holds the version its answer names', async () => {
  const h = harness()
  h.steers([pendingSteer('first')])
  let outcome: string | undefined
  const removal = h.client.cancelPendingSteer('first').then((answer) => {
    outcome = answer
  })
  h.receive({ type: 'remove_result', id: 'first', outcome: 'removed', at: { epoch: 'e', revision: 2 } })
  await settle()
  expect(outcome).toBeUndefined()
  h.steers([])
  await removal
  expect(outcome).toBe('removed')
})

// A gap in the agents' revisions is filled on the same socket. (It closed the
// socket.)
test('an agent change past the next revision asks for the changes since the one held and waits for them', () => {
  const h = harness()
  h.steers([])
  h.sent.length = 0
  h.receive({ type: 'agent', revision: 3, agent: { ...entry(ROOT, 'running'), pendingSteers: [pendingSteer('lost')] } })
  expect(h.sent).toEqual([{ type: 'agents_since', held: { epoch: 'e', revision: 1 } }])
  expect(h.client.pendingSteers()).toEqual([])
  h.receive({ type: 'agent', revision: 4, agent: entry(ROOT, 'running') })
  h.receive({
    type: 'agents',
    version: { epoch: 'e', revision: 4 },
    since: 1,
    agents: [{ ...entry(ROOT, 'running'), pendingSteers: [pendingSteer('kept')] }],
  })
  expect(h.client.pendingSteers()).toEqual([pendingSteer('kept')])
  expect(h.closes()).toBe(0)
  expect(h.events.map((event) => event.type)).not.toContain('disconnected')
})

test('the pending steers leave by id, even when the history arrives before the list', () => {
  const h = harness()
  const written = { ...pendingSteer('first'), type: 'steer' as const, createdAt }
  h.receive({ type: 'transcript_reset', node: ROOT, start: 0, length: 1, blocks: [written], version: { epoch: 'e', revision: 1 } })
  h.steers([pendingSteer('first'), pendingSteer('second')])
  expect(h.client.pendingSteers()).toEqual([pendingSteer('second')])
  // Both have the same content; only the id says which one the transcript holds.
  h.receive({ type: 'transcript_patch', node: ROOT, patches: [{ op: 'add', index: 1, value: { ...written, id: 'second' } }], revision: 2 })
  expect(h.client.pendingSteers()).toEqual([])
  expect(h.events.filter((event) => event.type === 'pending_steers').at(-1)).toEqual({ type: 'pending_steers', pendingSteers: [] })
})

// A turn that fails meanwhile is no answer to a removal.
test('a cancelled steer leaves with the list the server answers, and the removal resolves with the answer', async () => {
  const h = harness()
  h.steers([pendingSteer('first'), pendingSteer('second')])
  let outcome: string | undefined
  const removal = h.client.cancelPendingSteer('first').then((answer) => {
    outcome = answer
  })
  expect(h.sent).toEqual([{ type: 'cancel_pending_steer', steerId: 'first' }])
  h.receive({ type: 'error', code: 'generation_failed', message: 'The model failed' })
  h.steers([pendingSteer('second')])
  h.receive({ type: 'remove_result', id: 'other', outcome: 'removed', at: { epoch: 'e', revision: 2 } })
  await Promise.resolve()
  expect(outcome).toBeUndefined()
  expect(h.client.pendingSteers()).toEqual([pendingSteer('second')])
  h.receive({ type: 'remove_result', id: 'first', outcome: 'gone_on', at: { epoch: 'e', revision: 2 } })
  await removal
  expect(outcome).toBe('gone_on')
})

test('the pending steers the client answers are its own copies', () => {
  const h = harness()
  h.steers([pendingSteer('first')])
  const copy = h.client.pendingSteers()
  copy[0]!.content.length = 0
  copy.length = 0
  expect(h.client.pendingSteers()).toEqual([pendingSteer('first')])
})

test("the calls being written are the root's entry's until its next change, a subagent's are its own, and a whole agents frame starts over", () => {
  const h = harness()
  const writing = { toolUseId: 'call-1', toolName: 'shell', description: null }
  h.calls([writing])
  h.calls([{ ...writing, toolUseId: 'call-9' }], 'child')
  expect(h.client.pendingCalls()).toEqual([writing])
  h.calls([{ ...writing, description: 'Write the categorizer' }])
  expect(h.client.pendingCalls()[0]?.description).toBe('Write the categorizer')
  h.receive({ type: 'agents', version: { epoch: 'e2', revision: 1 }, agents: [entry(ROOT, 'idle')] })
  expect(h.client.pendingCalls()).toEqual([])
})

test('a frame the contract does not allow disconnects the client before anything acts on it', () => {
  for (const frame of [
    { type: 'agent', revision: 1, agent: { ...entry(ROOT, 'running'), pendingSteers: [{ id: 7 }] } },
    { type: 'transcript_reset', node: ROOT, start: 0, length: 1, blocks: [{ type: 'text' }], version: { epoch: 'e', revision: 0 } },
    { type: 'transcript_patch', node: ROOT, patches: [{ op: 'add', path: ['blocks', 0], value: text('a', 'x') }], revision: 1 },
    { type: 'tool_progress', toolUseId: 't1', output: [] },
  ]) {
    const h = harness()
    h.receiveValue(frame)
    expect(h.events.map((event) => event.type)).toEqual(['disconnected'])
    const [event] = h.events
    expect(event?.type === 'disconnected' && event.error.message).toStartWith('Invalid server frame')
    expect(h.blocks()).toEqual([])
    expect(h.client.pendingSteers()).toEqual([])
    expect(h.closes()).toBe(1)
  }
})

test('a patch at or before the client revision is ignored, and a gap asks for a fresh transcript', () => {
  const h = harness()
  h.receive({ type: 'transcript_reset', node: ROOT, start: 0, length: 1, blocks: [text('a', 'one')], version: { epoch: 'e', revision: 1 } })
  expect(h.client.transcriptVersion()).toEqual({ epoch: 'e', revision: 1 })
  h.receive({ type: 'transcript_patch', node: ROOT, patches: [{ op: 'append_text', index: 0, delta: ' again' }], revision: 1 })
  expect(h.blocks()).toEqual([text('a', 'one')])

  h.receive({ type: 'transcript_patch', node: ROOT, patches: [{ op: 'add', index: 1, value: text('c', 'three') }], revision: 3 })
  expect(h.sent).toEqual([{ type: 'subscribe', node: ROOT }])
  expect(h.client.transcriptVersion()).toBeNull()
  h.receive({ type: 'transcript_patch', node: ROOT, patches: [{ op: 'add', index: 1, value: text('d', 'four') }], revision: 4 })
  expect(h.sent).toEqual([{ type: 'subscribe', node: ROOT }])
  expect(h.blocks()).toEqual([text('a', 'one')])

  h.receive({
    type: 'transcript_reset',
    node: ROOT,
    start: 0,
    length: 3,
    blocks: [text('a', 'one'), text('b', 'two'), text('c', 'three')],
    version: { epoch: 'e', revision: 4 },
  })
  h.receive({ type: 'transcript_patch', node: ROOT, patches: [{ op: 'append_text', index: 2, delta: ' more' }], revision: 5 })
  expect(h.blocks().at(-1)).toMatchObject(text('c', 'three more'))
  expect(h.client.transcriptVersion()).toEqual({ epoch: 'e', revision: 5 })
})

test('each subscribed node\'s transcript follows the same revision rule, and a node the page does not follow is ignored', () => {
  const h = harness()
  h.receive({ type: 'transcript_reset', node: ROOT, start: 0, length: 1, blocks: [], version: { epoch: 'e', revision: 0 } })
  h.client.subscribeTo('child')
  h.receive({ type: 'transcript_reset', node: 'child', start: 0, length: 0, blocks: [], version: { epoch: 'c', revision: 3 } })
  h.receive({ type: 'transcript_patch', node: 'child', patches: [{ op: 'add', index: 0, value: text('x', 'x') }], revision: 4 })
  h.receive({ type: 'transcript_patch', node: 'child', patches: [{ op: 'append_text', index: 0, delta: ' again' }], revision: 4 })
  const patches = h.events.filter((event) => event.type === 'transcript_patch')
  expect(patches).toEqual([
    { type: 'transcript_patch', node: 'child', patches: [{ op: 'add', index: 0, value: text('x', 'x') }], version: { epoch: 'c', revision: 4 }, failures: {} },
  ])
  expect(h.client.transcriptVersion('child')).toEqual({ epoch: 'c', revision: 4 })
  h.receive({ type: 'transcript_patch', node: 'child', patches: [], revision: 6 })
  h.receive({ type: 'transcript_patch', node: 'unknown', patches: [], revision: 1 })
  expect(h.sent).toEqual([{ type: 'subscribe', node: 'child' }, { type: 'subscribe', node: 'child' }])
  expect(h.client.transcriptVersion('child')).toBeNull()
})

// A node the page stops showing streams no more
// (`crates/agent-server/src/server/tree.rs`); a frame of it already on its way
// is dropped, so it neither writes the transcript the page let go of nor asks
// for it again, and showing the node again takes its next reset.
test('a node the page stops showing drops its frames on their way until its reset', () => {
  const h = harness()
  h.client.subscribeTo('child')
  h.receive({ type: 'transcript_reset', node: 'child', start: 0, length: 0, blocks: [], version: { epoch: 'c', revision: 3 } })
  h.client.unsubscribeFrom('child')
  h.receive({ type: 'transcript_patch', node: 'child', patches: [], revision: 4 })
  h.receive({ type: 'transcript_patch', node: 'child', patches: [], revision: 6 })
  h.client.subscribeTo('child')
  h.receive({ type: 'transcript_reset', node: 'child', start: 0, length: 0, blocks: [], version: { epoch: 'c', revision: 7 } })
  h.receive({ type: 'transcript_patch', node: 'child', patches: [], revision: 8 })

  expect(h.sent).toEqual([
    { type: 'subscribe', node: 'child' },
    { type: 'unsubscribe', node: 'child' },
    { type: 'subscribe', node: 'child' },
  ])
  expect(h.events.filter((event) => event.type.startsWith('transcript')).map((event) => event.type)).toEqual([
    'transcript_reset',
    'transcript_reset',
    'transcript_patch',
  ])
})

test('closed clears the transcript, the version and the pending steers, and ends the actions', async () => {
  const h = harness()
  h.receive({ type: 'transcript_reset', node: ROOT, start: 0, length: 1, blocks: [text('a', 'one')], version: { epoch: 'e', revision: 1 } })
  h.steers([pendingSteer('first')])
  const action = h.client.send([{ type: 'text', text: 'hi' }])
  const steer = h.client.steer([{ type: 'text', text: 'also' }]).catch((error: unknown) => error)
  h.receive({ type: 'closed' })
  await action
  expect(await steer).toMatchObject({ message: 'Agent connection closed' })
  expect(h.blocks()).toEqual([])
  expect(h.client.transcriptVersion()).toBeNull()
  expect(h.client.pendingSteers()).toEqual([])
})

test('retry, resume and compact resolve when their action ends, and a busy session refuses them', async () => {
  const h = harness()
  const retry = h.client.retry()
  h.phase('running')
  h.phase('idle')
  await retry
  const resume = h.client.resume('resume-1')
  const compact = h.client.compact().catch((error: unknown) => error)
  h.phase('running')
  h.receive({ type: 'rejected', command: 'compact', code: 'session_busy', reason: 'Session is busy (running)' })
  expect(await compact).toMatchObject({ message: 'Session is busy (running)', code: 'session_busy' })
  h.phase('idle')
  await resume
  expect(h.sent).toEqual([{ type: 'retry' }, { type: 'resume', id: 'resume-1' }, { type: 'compact' }])
})

// A child's context card compacts that child: the frame names it, and the wait
// follows the child's phase, not the root's.
test('a compact of a subagent names it and ends when the child\'s compaction ends', async () => {
  const h = harness()
  let ended = false
  const compact = h.client.compact('child').then(() => {
    ended = true
  })
  expect(h.sent).toEqual([{ type: 'compact', node: 'child' }])
  h.phase('running')
  h.phase('idle')
  h.phase('compacting', 'child')
  await Promise.resolve()
  expect(ended).toBe(false)
  h.phase('idle', 'child')
  await compact
  expect(ended).toBe(true)
})

// A child's phase is the child's: its entry reaches the listeners and leaves
// the root's own phase, and what waits on it, alone.
test('a subagent\'s entry is passed on and ends no action of the root', async () => {
  const h = harness()
  let retried = false
  const retry = h.client.retry().then(() => {
    retried = true
  })
  h.phase('running')
  h.phase('idle', 'child')
  await settle()
  expect(retried).toBe(false)
  h.phase('idle')
  await retry
  expect(h.events.flatMap((event) => (event.type === 'agent' ? [[event.agent.id, event.agent.phase]] : []))).toEqual([
    ['child', 'idle'],
    [ROOT, 'idle'],
  ])
})

test('a failed turn fails its action with the error frame, and the aborts answer in request order', async () => {
  const h = harness()
  const action = h.client.send([{ type: 'text', text: 'hi' }]).catch((error: unknown) => error)
  h.phase('running')
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

  const other = harness({ opened: false })
  const opening = other.client.open({ root: ROOT }).catch((error: unknown) => error)
  other.end(new Error('The agent socket closed (4001 lagged)'))
  expect(await opening).toBeInstanceOf(Error)
  expect(other.events.at(-1)).toMatchObject({ type: 'disconnected', error: { message: 'The agent socket closed (4001 lagged)' } })
})

// The page counts a request's time on its own clock: the frame's arrival
// less how long the request had streamed, and its end at the arrival of the
// frame that says it ended, so a difference between the server's clock and
// the page's never shows in the speed.
test('a request\'s output is timed from the page\'s clock as its frames arrive', () => {
  const h = harness()
  const now = spyOn(Date, 'now').mockReturnValue(50_000)
  try {
    h.receive({ type: 'request_output', output: { requestId: 'r-1', tokens: 0, streamedMs: null, ended: false } })
    h.receive({ type: 'request_output', output: { requestId: 'r-1', tokens: 120, streamedMs: 3_000, ended: false } })
    now.mockReturnValue(52_000)
    h.receive({ type: 'request_output', output: { requestId: 'r-1', tokens: 96, streamedMs: 4_500, ended: true } })
    h.receive({ type: 'request_output', output: null })
  } finally {
    now.mockRestore()
  }
  expect(h.events.filter((event) => event.type === 'request_output')).toEqual([
    { type: 'request_output', output: { requestId: 'r-1', tokens: 0, firstTokenAt: null, endedAt: null } },
    { type: 'request_output', output: { requestId: 'r-1', tokens: 120, firstTokenAt: 47_000, endedAt: null } },
    { type: 'request_output', output: { requestId: 'r-1', tokens: 96, firstTokenAt: 47_500, endedAt: 52_000 } },
    { type: 'request_output', output: null },
  ])
})

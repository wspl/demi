import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { effectScope, nextTick, reactive } from 'vue'
import type { Block } from '@demicodes/protocol'
import { ACTIVITY_HANDOFF_MS } from '../activity-slot'
import { useMessageRows, type MessageRowsSource } from '../message-rows'
import { waitingAs, type MessageListBlock } from '../pending-steers'
import { rowKey } from '../row-key'
import { createdAt, model, userBlock } from './agent-harness'

// The rows the transcript list renders, followed through a message sent while
// the reader is at the end of a conversation (`product.md` § Following the
// agent's work, `runtime.md` § Work groups). The list draws what this gives
// it, so a row lost between two steps is a frame in which the list is
// shorter. Real heights and the frame they are drawn in are checked in the
// product. Pure, with a fake clock: a few milliseconds.

const TAIL_ROW = 'tail-row'

const text = (id: string, body: string): Block => ({ type: 'text', id, createdAt, model, text: body })
const thinking = (id: string): Block => ({ type: 'thinking', id, createdAt, model, text: '', signature: null })
const call = (id: string, status: 'executing' | 'completed'): Block => ({
  type: 'tool_call',
  id,
  createdAt,
  model,
  toolUseId: `${id}-use`,
  toolName: 'shell',
  status,
  input: JSON.stringify({ script: 'sleep 2 && echo done', description: 'Wait two seconds' }),
  output: [],
  view: null,
})

/** What the list draws: its rows by key, each wait of the tail row as one. */
function drawnRows(rows: readonly MessageListBlock[]): string[] {
  return rows.map((row) => row.type === 'activity_slot' ? TAIL_ROW : rowKey(row))
}

beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  jest.useRealTimers()
})

// The run starts with thinking, or with a call the model is still writing:
// either way its row is the same from that first step to the turn's end.
test.each(['thinking', 'call being written'] as const)('a sent message, its run from %s and its answer only add rows, keep the answer before them closed, and the run is one shimmering row', async (first) => {
  const state = reactive<MessageRowsSource>({
    conversationId: 'c-1',
    blocks: [userBlock('u-1', 'm-1', 'What is in the folder?'), text('a-1', 'Three files.')],
    pendingCalls: [],
    pendingSteers: [],
    queue: [],
    phase: 'idle',
    pendingSubmission: null,
    atStart: true,
    atEnd: true,
    latest: true,
  })
  const scope = effectScope()
  const list = scope.run(() => useMessageRows(() => state))!
  const drawn = () => drawnRows(list.rows.value)
  const run = () => list.rows.value.find((row) => row.type === 'work_group')
  let previous = drawn()
  let runKey: string | null = null

  async function step(name: string, change: () => void): Promise<void> {
    change()
    await nextTick()
    const now = drawn()
    // Each row drawn before is drawn still: the list never draws a frame
    // shorter, only to grow back. The tail row goes only as a row takes its place.
    const lost = previous.filter((key) => !now.includes(key))
    expect({ name, twice: now.filter((key, index) => now.indexOf(key) !== index) }).toEqual({ name, twice: [] })
    expect({ name, lost }).toEqual({ name, lost: lost.includes(TAIL_ROW) && now.length >= previous.length ? [TAIL_ROW] : [] })
    // The answer before the message keeps Copy and Fork while the message is delivered.
    expect({ name, footer: list.footerIds.value.has('a-1') }).toEqual({ name, footer: true })
    // The run is one row from its first step on.
    const key = run() ? rowKey(run()!) : null
    if (runKey !== null) {
      expect({ name, key }).toEqual({ name, key: runKey })
    }
    runKey = key ?? runKey
    previous = now
  }

  await step('sent', () => {
    state.pendingSubmission = { id: 'm-2', way: 'send', text: 'Run the command, then reply.', attachments: [], error: null }
  })
  expect(drawn().at(-1)).toBe(TAIL_ROW)
  await step('running', () => {
    state.phase = 'running'
  })
  await step('written', () => {
    state.blocks = [...state.blocks, userBlock('u-2', 'm-2', 'Run the command, then reply.')]
  })
  await step('confirmed', () => {
    state.pendingSubmission = null
  })
  const writing = (description: string | null) => [{ toolUseId: 'c-1-use', toolName: 'shell', description }]
  await step(first, () => {
    if (first === 'thinking') {
      state.blocks = [...state.blocks, thinking('k-1')]
    } else {
      state.pendingCalls = writing(null)
    }
  })
  await step('handed over', () => {
    jest.advanceTimersByTime(ACTIVITY_HANDOFF_MS)
  })
  expect(runKey).not.toBeNull()
  await step('call being written', () => {
    state.pendingCalls = writing('Wait two seconds')
  })
  // The call's block comes before the page drops the call being written.
  await step('command', () => {
    state.blocks = [...state.blocks, call('c-1', 'executing')]
  })
  await step('call written', () => {
    state.pendingCalls = []
  })
  await step('command ended', () => {
    state.blocks = state.blocks.map((block) => block.id === 'c-1' ? call('c-1', 'completed') : block)
  })
  // While the model is waited for, the run's row is the turn's activity: it
  // shimmers, and no Requesting row shows under it.
  expect(run()?.live).toBe(true)
  expect(list.slot.value).toBeNull()
  await step('answer', () => {
    state.blocks = [...state.blocks, text('a-2', 'Done.')]
  })
  expect(run()?.live).toBe(false)
  await step('idle', () => {
    state.phase = 'idle'
  })
  expect(list.footerIds.value.has('a-2')).toBe(true)
  scope.stop()
})

/** A conversation whose agent runs: the user's message, then a run whose last call ended. */
function running(): MessageRowsSource {
  return reactive<MessageRowsSource>({
    conversationId: 'c-1',
    blocks: [userBlock('u-1', 'm-1', 'Fix the login test.'), call('c-1', 'completed')],
    pendingCalls: [{ toolUseId: 'c-2-use', toolName: 'shell', description: 'Run the login test' }],
    pendingSteers: [],
    queue: [],
    phase: 'running',
    pendingSubmission: null,
    atStart: true,
    atEnd: true,
    latest: true,
  })
}

// A steer shows what it is from the moment it is sent (`product.md` § Steer
// or queue): its line under it, in the row the steer the session holds
// keeps, so the row neither pops its line in late nor grows; the line goes
// only once the turn reads the steer.
test('a steer on its way has its line from the send, and keeps its row and its line until the turn reads it', async () => {
  const state = running()
  const scope = effectScope()
  const list = scope.run(() => useMessageRows(() => state))!
  const row = () => list.rows.value.find((block) => rowKey(block) === 'message:s-1')
  const seen: { name: string; index: number; line: string | undefined }[] = []
  async function step(name: string, change: () => void): Promise<void> {
    change()
    await nextTick()
    const found = row()
    seen.push({ name, index: found ? list.rows.value.indexOf(found) : -1, line: found ? waitingAs(found) : undefined })
  }
  const content = [{ type: 'text' as const, text: 'Also check the sign-up page.' }]
  await step('sent', () => {
    state.pendingSubmission = { id: 's-1', way: 'steer', text: 'Also check the sign-up page.', attachments: [], error: null }
  })
  // The session lists the steer before the page lets the message go.
  await step('listed', () => {
    state.pendingSteers = [{ id: 's-1', content }]
  })
  await step('confirmed', () => {
    state.pendingSubmission = null
  })
  expect(seen).toEqual([
    { name: 'sent', index: 2, line: 'steer' },
    { name: 'listed', index: 2, line: 'steer' },
    { name: 'confirmed', index: 2, line: 'steer' },
  ])
  // The turn reads it at its next step: the row is the steer written.
  await step('read', () => {
    state.pendingSteers = []
    state.blocks = [...state.blocks, { type: 'steer', id: 's-1', turnId: 't-1', createdAt, model, content }]
  })
  expect(row()?.type).toBe('steer')
  expect(seen.at(-1)?.line).toBeUndefined()
  scope.stop()
})

test('a queued message on its way is counted under the queue divider from the send, in the row it keeps', async () => {
  const state = running()
  const scope = effectScope()
  const list = scope.run(() => useMessageRows(() => state))!
  state.pendingSubmission = { id: 'q-1', way: 'queue', text: 'Then deploy it.', attachments: [], error: null }
  await nextTick()
  const shown = () => list.rows.value.map((block) => block.type === 'queue_divider' ? `divider:${block.count}` : `${rowKey(block)}:${waitingAs(block) ?? ''}`)
  const before = shown()
  expect(before.slice(-2)).toEqual(['divider:1', 'message:q-1:queued'])
  state.queue = [{ id: 'q-1', content: [{ type: 'text', text: 'Then deploy it.' }] }]
  state.pendingSubmission = null
  await nextTick()
  expect(shown()).toEqual(before)
  scope.stop()
})

// Messages that wait stay out of the turn's flow (`product.md` § Following
// the agent's work): what the turn does is read from the transcript alone.
test('a steer waiting under a run the model is still writing adds no Requesting row', async () => {
  const state = running()
  state.pendingSteers = [{ id: 's-1', content: [{ type: 'text', text: 'Also check the sign-up page.' }] }]
  const scope = effectScope()
  const list = scope.run(() => useMessageRows(() => state))!
  await nextTick()
  expect(drawnRows(list.rows.value)).toEqual(['message:m-1', 'work:c-1-use', 'message:s-1'])
  expect(list.slot.value).toBeNull()
  scope.stop()
})

test('while the model is waited for, the Requesting row stands right after the transcript, above the steers and the queue', async () => {
  const state = running()
  state.blocks = [userBlock('u-1', 'm-1', 'Fix the login test.')]
  state.pendingCalls = []
  state.pendingSteers = [{ id: 's-1', content: [{ type: 'text', text: 'Also check the sign-up page.' }] }]
  state.queue = [{ id: 'q-1', content: [{ type: 'text', text: 'Then deploy it.' }] }]
  const scope = effectScope()
  const list = scope.run(() => useMessageRows(() => state))!
  await nextTick()
  expect(drawnRows(list.rows.value)).toEqual(['message:m-1', TAIL_ROW, 'message:s-1', 'queue-divider', 'message:q-1'])
  scope.stop()
})

// A command's notice and an agent's receipt that arrive while the agent works
// are steps of its run (`runtime.md` § Work groups): the run stays one row
// under its name, the turn's activity, with no Requesting row under it.
test('a notice and a receipt join the running run, which stays one row under one name', async () => {
  const state = running()
  state.pendingCalls = []
  const scope = effectScope()
  const list = scope.run(() => useMessageRows(() => state))!
  const drawn = () => drawnRows(list.rows.value)
  const before = drawn()
  expect(before).toEqual(['message:m-1', 'work:c-1-use'])

  state.blocks = [...state.blocks, {
    type: 'wakeup',
    id: 'n-1',
    turnId: 'n-1',
    createdAt,
    model,
    placement: 'steer',
    reports: [{ commandId: '17', title: 'Build the app', event: { kind: 'ended', exitCode: 0 }, output: '' }],
  }]
  await nextTick()
  expect(drawn()).toEqual(before)
  state.blocks = [...state.blocks, {
    type: 'agent_message',
    id: 'm-2',
    turnId: 'n-1',
    createdAt,
    model,
    message: {
      id: 'm-2',
      sender: { id: 'ag-2', number: 2, description: 'Check the logout path', round: 1 },
      recipientId: 'c-1',
      timestamp: createdAt,
      content: 'Halfway there.',
      event: { type: 'message' },
    },
  }]
  await nextTick()
  expect(drawn()).toEqual(before)
  const run = list.rows.value.at(-1)
  expect(run?.type === 'work_group' && run.live && run.steps.map((step) => step.id)).toEqual(['c-1', 'n-1', 'm-2'])
  expect(list.slot.value).toBeNull()
  scope.stop()
})

import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { effectScope, nextTick, reactive } from 'vue'
import type { Block } from '@demicodes/protocol'
import { ACTIVITY_HANDOFF_MS } from '../activity-slot'
import { useMessageRows, type MessageRowsSource } from '../message-rows'
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
  /** What the list draws: its rows by key, then the tail row. */
  const drawn = () => [...list.rows.value.map(rowKey), ...(list.slot.value ? [TAIL_ROW] : [])]
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
    state.pendingSubmission = { id: 'm-2', text: 'Run the command, then reply.', attachments: [], error: null }
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

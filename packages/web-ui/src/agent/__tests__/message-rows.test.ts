import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { effectScope, nextTick, reactive } from 'vue'
import type { Block } from '@demicodes/protocol'
import { addPage, EMPTY_TRANSCRIPT, heldBlocks } from '@demicodes/conversation-client'
import { useMessageRows, type MessageRows, type MessageRowsSource } from '../message-rows'
import { waitingAs, type MessageListBlock } from '../pending-steers'
import { rowKey } from '../row-key'
import { createdAt, model, userBlock } from './agent-harness'

// The rows the transcript list renders, followed through a message sent while
// the reader is at the end of a conversation (`product.md` § Following the
// agent's work, `runtime.md` § Work groups). The list draws what this gives
// it, so a row lost between two steps is a frame in which the list is
// shorter. Real heights and the frame they are drawn in are checked in the
// product. Pure, with a fake clock: a few milliseconds.

const text = (id: string, body: string): Block => ({ type: 'text', id, createdAt, model, text: body })
/** A step of the run `run`, the id of its first block, as the backend stores it. */
const thinking = (id: string, run = id): Block => ({ type: 'thinking', id, run, createdAt, model, text: '', signature: null })
const call = (id: string, status: 'executing' | 'completed', run = id): Block => ({
  type: 'tool_call',
  id,
  run,
  createdAt,
  model,
  toolUseId: `${id}-use`,
  toolName: 'shell',
  status,
  input: JSON.stringify({ script: 'sleep 2 && echo done', description: 'Wait two seconds' }),
  output: [],
  view: null,
})

/** What the list draws: its rows by the keys it draws them under. */
function drawnRows(list: MessageRows): string[] {
  return list.rows.value.map(list.keyOf)
}

beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  jest.useRealTimers()
})

const abort = (id: string, isResumed = false): Block => ({ type: 'abort', id, createdAt, model, isResumed })
const failure = (id: string): Block => ({ type: 'error', id, createdAt, model, message: 'The provider request failed.', code: null })
const resumed = (id: string): Block => ({ type: 'resume', id, turnId: 't-1', createdAt, model, reason: null })

/** A conversation whose agent is idle after answering `m-1`. */
function answered(): MessageRowsSource {
  return reactive<MessageRowsSource>({
    conversationId: 'c-1',
    blocks: [userBlock('u-1', 'm-1', 'What is in the folder?'), text('a-1', 'Three files.')],
    pendingCalls: [],
    pendingSteers: [],
    queue: [],
    phase: 'idle',
    pendingSubmissions: [],
    atStart: true,
    atEnd: true,
    latest: true,
  })
}

/**
 * Follows the rows the list draws over the changes the page receives, in
 * the order it receives them. Whatever replaces a row takes its key, so a
 * row drawn before is drawn still after any change the user did not make
 * (`product.md` § Following the agent's work: what replaces a row takes its
 * place), and no key is drawn twice. `gives` names the keys a change the
 * user asked for lets go, as the Requesting row of a Continue gives way to
 * a step that joins the run above it.
 */
function follow(state: MessageRowsSource) {
  const scope = effectScope()
  const list = scope.run(() => useMessageRows(() => state))!
  let previous = drawnRows(list)
  const step = async (name: string, change: () => void, gives: string[] = []): Promise<string[]> => {
    change()
    await nextTick()
    const now = drawnRows(list)
    expect({ name, twice: now.filter((key, index) => now.indexOf(key) !== index) }).toEqual({ name, twice: [] })
    expect({ name, lost: previous.filter((key) => !now.includes(key)) }).toEqual({ name, lost: gives })
    previous = now
    return now
  }
  return { list, step, stop: () => scope.stop() }
}

// The run starts with thinking, or with a call the model is still writing:
// either way its row is the same from that first step to the turn's end.
test.each(['thinking', 'call being written'] as const)('a sent message, its run from %s and its answer only add rows, keep the answer before them closed, and the run is one shimmering row in the tail row\'s place', async (first) => {
  const state = answered()
  const { list, step: follows, stop } = follow(state)
  const run = () => list.rows.value.find((row) => row.type === 'work_group')
  let tailKey: string | null = null
  async function step(name: string, change: () => void): Promise<void> {
    await follows(name, change)
    // The answer before the message keeps Copy and Fork while the message is delivered.
    expect({ name, footer: list.footerIds.value.has('a-1') }).toEqual({ name, footer: true })
    // The tail row, then the run that takes its place, is one row.
    const key = list.keyOf(list.rows.value.at(-1)!)
    if (tailKey !== null && list.rows.value.at(-1)?.type !== 'text') {
      expect({ name, key }).toEqual({ name, key: tailKey })
    }
  }

  await step('sent', () => {
    state.pendingSubmissions = [{ id: 'm-2', way: 'send', text: 'Run the command, then reply.', attachments: [], error: null }]
  })
  expect(list.rows.value.at(-1)?.type).toBe('activity_slot')
  tailKey = list.keyOf(list.rows.value.at(-1)!)
  await step('running', () => {
    state.phase = 'running'
  })
  await step('written', () => {
    state.blocks = [...state.blocks, userBlock('u-2', 'm-2', 'Run the command, then reply.')]
  })
  await step('confirmed', () => {
    state.pendingSubmissions = []
  })
  const writing = (description: string | null) => [{ toolUseId: 'c-1-use', toolName: 'shell', description }]
  await step(first, () => {
    if (first === 'thinking') {
      state.blocks = [...state.blocks, thinking('k-1')]
    } else {
      state.pendingCalls = writing(null)
    }
  })
  // A call shows from its title: without it, the tail row stays.
  expect(run() !== undefined).toBe(first === 'thinking')
  await step('call being written', () => {
    state.pendingCalls = writing('Wait two seconds')
  })
  expect(run()).toBeDefined()
  // The call's block comes before the page drops the call being written.
  // The run starts at its first block: the thinking, or the call's own.
  const runOf = first === 'thinking' ? 'k-1' : 'c-1'
  await step('command', () => {
    state.blocks = [...state.blocks, call('c-1', 'executing', runOf)]
  })
  await step('call written', () => {
    state.pendingCalls = []
  })
  await step('command ended', () => {
    state.blocks = state.blocks.map((block) => block.id === 'c-1' ? call('c-1', 'completed', runOf) : block)
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
  stop()
})

/** A change the page receives, or a user's action, by what it is called. */
type Change = [name: string, change: (state: MessageRowsSource) => void]

const append = (...blocks: Block[]) => (state: MessageRowsSource) => {
  state.blocks = [...state.blocks, ...blocks]
}
const replace = (id: string, block: Block) => (state: MessageRowsSource) => {
  state.blocks = state.blocks.map((each) => each.id === id ? block : each)
}
const drop = (id: string) => (state: MessageRowsSource) => {
  state.blocks = state.blocks.filter((each) => each.id !== id)
}
const phase = (next: MessageRowsSource['phase']) => (state: MessageRowsSource) => {
  state.phase = next
}
/** The user presses Continue or Resume: the page knows at once. */
const recover: Change = ['Continue or Resume', (state) => {
  state.pendingAction = 'resume'
}]
/** The server acknowledges the recovery with the phase, before the transcript says so. */
const acknowledged: Change = ['acknowledged', (state) => {
  state.phase = 'running'
  state.pendingAction = null
}]
const sent: Change = ['sent', (state) => {
  state.pendingSubmissions = [{ id: 'm-2', way: 'send', text: 'Go on.', attachments: [], error: null }]
}]
const sentAndWritten: Change[] = [
  sent,
  ['running', phase('running')],
  ['written', append(userBlock('u-2', 'm-2', 'Go on.'))],
  ['confirmed', (state) => {
    state.pendingSubmissions = []
  }],
]

/** The run the stopped or failed turn had done: the user's message and a command that ended. */
const workedOn = (...end: Block[]): Block[] => [userBlock('u-1', 'm-1', 'Fix the login test.'), call('c-1', 'completed'), ...end]

// Each way a row gives way to the next state without the user removing it
// (`product.md` § Following the agent's work), as the events reach the page:
// the row that showed the old state stays drawn, under its key, and the row
// at the end keeps its key from `from` on.
const transitions: { name: string; blocks: Block[]; phase?: MessageRowsSource['phase']; from: string; changes: Change[] }[] = [
  {
    name: 'Stop, then Continue',
    blocks: workedOn(),
    phase: 'running',
    from: 'stopped',
    changes: [
      ['stopped', append(abort('x-1'))],
      ['idle', phase('idle')],
      recover,
      acknowledged,
      ['marked resumed', replace('x-1', abort('x-1', true))],
      ['resume written', append(resumed('r-1'))],
    ],
  },
  {
    name: 'an error at the end, then Resume',
    blocks: workedOn(failure('e-1')),
    from: 'start',
    changes: [
      recover,
      acknowledged,
      ['error dropped', drop('e-1')],
      ['resume written', append(resumed('r-1'))],
    ],
  },
  {
    name: 'Stop after the agent\'s words, then Continue, then the next step',
    blocks: [userBlock('u-1', 'm-1', 'Fix the login test.'), text('a-1', 'Looking.'), abort('x-1')],
    from: 'start',
    changes: [
      recover,
      acknowledged,
      ['marked resumed', replace('x-1', abort('x-1', true))],
      ['resume written', append(resumed('r-1'))],
      ['call', append(call('c-1', 'executing'))],
    ],
  },
  {
    name: 'a turn that failed before any output, then Resume, which runs it again',
    blocks: [userBlock('u-1', 'm-1', 'Fix the login test.'), failure('e-1')],
    from: 'start',
    changes: [
      recover,
      acknowledged,
      ['error dropped', drop('e-1')],
      ['thinking', append(thinking('k-1'))],
    ],
  },
  {
    name: 'a refused Continue',
    blocks: workedOn(abort('x-1')),
    from: 'start',
    changes: [
      recover,
      ['refused', (state) => {
        state.pendingAction = null
      }],
    ],
  },
  {
    name: 'a refused Resume',
    blocks: workedOn(failure('e-1')),
    from: 'start',
    changes: [
      recover,
      ['refused', (state) => {
        state.pendingAction = null
      }],
    ],
  },
  {
    name: 'Requesting, then the answer',
    blocks: [userBlock('u-1', 'm-1', 'Hello.')],
    phase: 'running',
    from: 'start',
    changes: [['answer', append(text('a-1', 'Hi.'))]],
  },
  {
    name: 'Requesting, then a failure',
    blocks: [userBlock('u-1', 'm-1', 'Hello.')],
    phase: 'running',
    from: 'start',
    changes: [
      ['failed', append(failure('e-1'))],
      ['idle', phase('idle')],
    ],
  },
  {
    name: 'Requesting, then a Stop',
    blocks: [userBlock('u-1', 'm-1', 'Hello.')],
    phase: 'running',
    from: 'start',
    changes: [
      ['stopped', append(abort('x-1'))],
      ['idle', phase('idle')],
    ],
  },
  {
    name: 'thinking without text, then the answer',
    blocks: [userBlock('u-1', 'm-1', 'Hello.'), thinking('k-1')],
    phase: 'running',
    from: 'start',
    changes: [['answer', append(text('a-1', 'Hi.'))]],
  },
  {
    name: 'Compacting, then the divider',
    blocks: [userBlock('u-1', 'm-1', 'Hello.'), text('a-1', 'Hi.')],
    from: 'compacting',
    changes: [
      ['compacting', phase('compacting')],
      ['divider', (state) => {
        state.blocks = [
          { type: 'compaction_boundary', id: 'b-1', createdAt, model, summary: 'The user said hello.', summaryTokens: 12 },
          ...state.blocks,
          { type: 'compaction_marker', id: 'cm-1', createdAt, model, boundaryId: 'b-1', compactedTokens: 400 },
        ]
        state.phase = 'idle'
      }],
    ],
  },
  {
    name: 'a send whose delivery fails, then Retry',
    blocks: [userBlock('u-1', 'm-1', 'Hello.'), text('a-1', 'Hi.')],
    from: 'sent',
    changes: [
      sent,
      ['not delivered', (state) => {
        state.pendingSubmissions = state.pendingSubmissions!.map((each) => ({ ...each, error: 'The connection was lost.' }))
      }],
      ['Retry', (state) => {
        state.pendingSubmissions = state.pendingSubmissions!.map((each) => ({ ...each, error: null }))
      }],
      ...sentAndWritten.slice(1),
    ],
  },
  {
    name: 'a failure told at the tail, then the session opened again',
    blocks: [userBlock('u-1', 'm-1', 'Hello.')],
    phase: 'running',
    from: 'failed',
    changes: [
      ['failed', (state) => {
        state.failure = { label: 'The conversation could not be opened.', retry: true }
        state.load = 'failed'
      }],
      ['opened again', (state) => {
        state.failure = null
        state.load = 'ready'
      }],
    ],
  },
]

test.each(transitions)('$name: no row goes while another takes its place, and the row at the end keeps its key', async ({ blocks, phase: initial, from, changes }) => {
  const state = answered()
  state.blocks = blocks
  state.phase = initial ?? 'idle'
  const { list, step, stop } = follow(state)
  const tail = () => list.keyOf(list.rows.value.at(-1)!)
  let kept = from === 'start' ? tail() : null
  for (const [name, change] of changes) {
    await step(name, () => change(state))
    if (name === from) {
      kept = tail()
    } else if (kept !== null) {
      expect({ name, tail: tail() }).toEqual({ name, tail: kept })
    }
  }
  stop()
})

// Continue after a Stop and Resume after an error: the Stopped row or the
// error record rolls into Requesting, and the turn's first step joins the
// run above, as the backend stores it (`runtime.md` § Work groups): the
// run's row keeps its key and shows the step as its newest, and the
// Requesting row goes, once, the one shrink the user asked for.
test.each([
  { name: 'Continue after a Stop', record: abort('x-1'), unwind: replace('x-1', abort('x-1', true)) },
  { name: 'Resume after an error', record: failure('e-1'), unwind: drop('e-1') },
])('$name: the next step joins the run above, and the Requesting row goes once', async ({ record, unwind }) => {
  const state = answered()
  state.blocks = workedOn(record)
  const { list, step, stop } = follow(state)
  const runRow = () => list.rows.value.filter((row) => row.type === 'work_group')
  const run = list.keyOf(runRow()[0]!)
  const tail = list.keyOf(list.rows.value.at(-1)!)
  await step('pressed', recover[1].bind(null, state))
  expect(list.rows.value.at(-1)?.type).toBe('activity_slot')
  expect(list.keyOf(list.rows.value.at(-1)!)).toBe(tail)
  await step('acknowledged', acknowledged[1].bind(null, state))
  await step('unwound', () => unwind(state))
  await step('resume written', () => append(resumed('r-1'))(state))
  expect(list.rows.value.at(-1)?.type).toBe('activity_slot')
  await step('call', () => append(call('c-2', 'executing', 'c-1'))(state), [tail])
  const runs = runRow()
  expect(runs.map((row) => row.type === 'work_group' && [row.live, row.steps.map((each) => each.id)])).toEqual([[true, ['c-1', 'c-2']]])
  expect(list.keyOf(runs[0]!)).toBe(run)
  expect(list.rows.value.at(-1)).toBe(runs[0])
  await step('answer', () => {
    replace('c-2', call('c-2', 'completed', 'c-1'))(state)
    append(text('a-2', 'Fixed.'))(state)
  })
  await step('ended', phase('idle').bind(null, state))
  stop()
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
    pendingSubmissions: [],
    atStart: true,
    atEnd: true,
    latest: true,
  })
}

// A call being written shows from its title (`runtime.md` § Calls being
// written): until the model has written its description, the tail row goes
// on saying Requesting, counting from the wait's start, and a run goes on
// showing its newest step; then the face rolls to the call's row, in the
// same row.
test('a call being written without its description leaves the face as it was, then rolls to its title', async () => {
  const opened = (description: string | null) => [{ toolUseId: 'c-2-use', toolName: 'shell', description }]

  const asked = answered()
  asked.blocks = [...asked.blocks, userBlock('u-2', 'm-2', 'Sort the downloads into folders.')]
  asked.phase = 'running'
  asked.waitingSince = Date.now() - 480_000
  const first = follow(asked)
  const requesting = first.list.keyOf(first.list.rows.value.at(-1)!)
  // The call's first output ends the wait the server counts.
  await first.step('call opened', () => {
    asked.pendingCalls = opened(null)
    asked.waitingSince = null
  })
  expect(first.list.rows.value.at(-1)?.type).toBe('activity_slot')
  expect(first.list.slot.value).toEqual({ kind: 'requesting', since: Date.now() - 480_000 })
  await first.step('title written', () => {
    asked.pendingCalls = opened('Write the categorizer')
  })
  const call = first.list.rows.value.at(-1)!
  expect(call.type === 'work_group' && call.steps.map((each) => each.id)).toEqual(['pending-call:c-2-use'])
  expect(first.list.keyOf(call)).toBe(requesting)
  expect(first.list.slot.value).toBeNull()
  first.stop()

  const busy = running()
  busy.pendingCalls = opened(null)
  const next = follow(busy)
  const run = () => next.list.rows.value.find((row) => row.type === 'work_group')
  const before = run()!
  expect(before.type === 'work_group' && before.steps.map((each) => each.id)).toEqual(['c-1'])
  expect(next.list.slot.value).toBeNull()
  await next.step('title written', () => {
    busy.pendingCalls = opened('Run the login test')
  })
  const after = run()!
  expect(after.type === 'work_group' && after.steps.map((each) => each.id)).toEqual(['c-1', 'pending-call:c-2-use'])
  expect(next.list.keyOf(after)).toBe(next.list.keyOf(before))
  next.stop()
})

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
    state.pendingSubmissions = [{ id: 's-1', way: 'steer', text: 'Also check the sign-up page.', attachments: [], error: null }]
  })
  // The session lists the steer before the page lets the message go.
  await step('listed', () => {
    state.pendingSteers = [{ id: 's-1', content }]
  })
  await step('confirmed', () => {
    state.pendingSubmissions = []
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

// A steer that will start the turn is drawn as the message it becomes
// (`product.md` § Following the agent's work): the message before it was not
// delivered, nothing runs, and the steer goes as the user's next message.
test('a steer on its way that comes to start the turn reads as the next message from then, in the row it keeps', async () => {
  const state = answered()
  const { list, step, stop } = follow(state)
  const shown = () => list.rows.value.flatMap((row) => row.type === 'activity_slot'
    ? [`tail:${row.slot.kind}`]
    : rowKey(row) === 'message:m-3' ? [`m-3:${waitingAs(row) ?? 'message'}`] : [])
  await step('sent', () => {
    state.pendingSubmissions = [{ id: 'm-2', way: 'send', text: 'Rename the folder.', attachments: [], error: null }]
  })
  await step('steered', () => {
    state.pendingSubmissions = [...state.pendingSubmissions!, { id: 'm-3', way: 'steer', text: 'Then list it.', attachments: [], error: null }]
  })
  expect(shown()).toEqual(['tail:requesting', 'm-3:steer'])
  await step('not delivered', () => {
    state.pendingSubmissions = state.pendingSubmissions!.map((each) => each.id === 'm-2' ? { ...each, error: 'The connection was lost.' } : each)
  })
  expect(shown()).toEqual(['m-3:message', 'tail:requesting'])
  stop()
})

test('a queued message on its way is counted under the queue divider from the send, in the row it keeps', async () => {
  const state = running()
  const scope = effectScope()
  const list = scope.run(() => useMessageRows(() => state))!
  state.pendingSubmissions = [{ id: 'q-1', way: 'queue', text: 'Then deploy it.', attachments: [], error: null }]
  await nextTick()
  const shown = () => list.rows.value.map((block) => block.type === 'queue_divider' ? `divider:${block.count}` : `${rowKey(block)}:${waitingAs(block) ?? ''}`)
  const before = shown()
  expect(before.slice(-2)).toEqual(['divider:1', 'message:q-1:queued'])
  state.queue = [{ id: 'q-1', content: [{ type: 'text', text: 'Then deploy it.' }] }]
  state.pendingSubmissions = []
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
  expect(drawnRows(list)).toEqual(['message:m-1', 'work:c-1-use', 'message:s-1'])
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
  expect(drawnRows(list)).toEqual(['message:m-1', 'tail-row:1', 'message:s-1', 'queue-divider', 'message:q-1'])
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
  const drawn = () => drawnRows(list)
  const before = drawn()
  expect(before).toEqual(['message:m-1', 'work:c-1-use'])

  state.blocks = [...state.blocks, {
    type: 'wakeup',
    id: 'n-1',
    run: 'c-1',
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
    run: 'c-1',
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

/**
 * The rows a list draws of a finished turn, by key, a run with the ids of
 * the steps it shows.
 */
function finishedRows(blocks: Block[]): string[] {
  const scope = effectScope()
  const list = scope.run(() => useMessageRows(() => ({ ...answered(), blocks })))!
  const rows = list.rows.value.map((row) =>
    row.type === 'work_group' ? `${list.keyOf(row)} [${row.steps.map((step) => step.id).join(' ')}]` : list.keyOf(row))
  scope.stop()
  return rows
}

/** `blocks` as a page read after a reload holds them: each in its light form (`web-api.md` § Light form). */
function reopened(blocks: Block[]): Block[] {
  const light = blocks.map((block): Block =>
    block.type === 'thinking' ? { ...block, text: '', signature: null, textLength: [...block.text.trim()].length } : block)
  return heldBlocks(addPage(EMPTY_TRANSCRIPT, { start: 0, length: light.length, blocks: light }, false))
}

const thought = (id: string, body: string, run = id): Block => ({ type: 'thinking', id, run, createdAt, model, text: body, signature: 'sig' })

// A page reopened reads the transcript in pages, whose thinking comes
// without its text until its row is opened: the turn shows the same rows as
// it did live, each thinking with text among them, and a thinking without
// text, as Gemini signs before each call, or of white space only, nowhere.
test.each([
  ['thinking before the answer', [thought('k-1', 'The folder is the workspace.'), text('a-2', 'Three files.')], 'work:k-1 [k-1]'],
  ['thinking before a call', [thought('k-1', 'List the folder first.'), call('c-1', 'completed', 'k-1'), text('a-2', 'Three files.')], 'work:k-1 [k-1 c-1]'],
  ['two thoughts before the answer', [thought('k-1', 'The folder is the workspace.'), thought('k-2', 'Count its files.', 'k-1'), text('a-2', 'Three files.')], 'work:k-2'],
  ['textless thinking before a call', [thought('k-1', ''), call('c-1', 'completed', 'k-1'), text('a-2', 'Three files.')], 'work:k-1 [c-1]'],
  ['white-space-only thinking before a call', [thought('k-1', ' \n'), call('c-1', 'completed', 'k-1'), text('a-2', 'Three files.')], 'work:k-1 [c-1]'],
] as const)('a turn with %s shows the same rows after the page is reopened', (_name, turn, run) => {
  const blocks = [userBlock('u-2', 'm-2', 'What is in the folder?'), ...turn]
  const live = finishedRows(blocks)
  expect(live).toContain(run)
  expect(finishedRows(reopened(blocks))).toEqual(live)
})

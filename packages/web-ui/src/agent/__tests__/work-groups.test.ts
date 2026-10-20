import { describe, expect, test } from 'bun:test'
import type { Block } from '@demicodes/protocol'
import type { MessageListBlock } from '../pending-steers'
import { followedBlockId, groupWork, runLabel, runningOn, workSummary, type WorkGroupBlock } from '../work-groups'
import { isThinkingBlockStreaming } from '../block-streaming'
import { rowKey } from '../row-key'
import { createdAt, model } from './agent-harness'

// Cost: pure, a few milliseconds for the file.

const thinking = (id: string, text = ''): Block => ({ type: 'thinking', id, createdAt, model, text, signature: null })
const text = (id: string): Block => ({ type: 'text', id, createdAt, model, text: 'Done.' })
const call = (
  id: string,
  toolName = 'shell',
  input: Record<string, unknown> = { script: 'true', description: `Run ${id}` },
  status: Extract<Block, { type: 'tool_call' }>['status'] = 'completed',
): Block => ({
  type: 'tool_call',
  id,
  createdAt,
  model,
  toolUseId: `${id}-use`,
  toolName,
  status,
  input: JSON.stringify(input),
  endedAt: status === 'executing' ? null : createdAt,
  output: [],
  view: null,
})
const check = (id: string, ...commandIds: number[]): Block =>
  call(id, 'shell', { script: `demi shell status ${commandIds.join(' ')}`, description: `Check ${id}` })
const writing = (id: string, toolName = 'shell'): MessageListBlock => ({
  type: 'pending_call',
  id: `pending-call:${id}-use`,
  call: { toolUseId: `${id}-use`, toolName, description: `Write ${id}` },
})
const steps = (...blocks: (Block | MessageListBlock)[]) => blocks as WorkGroupBlock['steps']
type CommandEvent = Extract<Block, { type: 'wakeup' }>['reports'][number]['event']
/**
 * A command's notice: the end of each command it names, each as `event` says it
 * ended.
 */
const ended = (id: string, event: CommandEvent, ...commandIds: string[]): Block => ({
  type: 'wakeup',
  id,
  turnId: 'turn-1',
  createdAt,
  model,
  placement: 'steer',
  reports: commandIds.map((commandId) => ({ commandId, title: `Run ${commandId}`, event, output: '' })),
})
/** A command's notice: each command it names finished. */
const notice = (id: string, ...commandIds: string[]): Block => ended(id, { kind: 'ended', exitCode: 0 }, ...commandIds)
/** A receipt from another agent: an update, or its end. */
const receipt = (id: string, agent: string, event: Extract<Block, { type: 'agent_message' }>['message']['event'] = { type: 'message' }): Block => ({
  type: 'agent_message',
  id,
  turnId: 'turn-1',
  createdAt,
  model,
  message: {
    id,
    sender: { id: agent, number: 2, description: `Check ${agent}`, round: 1 },
    recipientId: 'conversation-1',
    timestamp: createdAt,
    content: 'Halfway there.',
    event,
  },
})
const steer = (id: string): Block => ({ type: 'steer', id, turnId: 'turn-1', createdAt, model, content: [{ type: 'text', text: 'Also check the logout.' }] })
const allowed = (id: string): Block => ({
  type: 'agent_message',
  id,
  turnId: 'turn-1',
  createdAt,
  model,
  message: { id, recipientId: 'conversation-1', timestamp: createdAt, content: 'Allowed.', event: { type: 'permission', outcome: 'allowed', action: 'manage skills' } },
})

/** Each row as its id, a group as the ids of its steps. */
function rows(list: readonly MessageListBlock[]): (string | string[])[] {
  return list.map((row) => (row.type === 'work_group' ? row.steps.map((step) => step.id) : row.id))
}

/** `blocks` as steps of the run `first`, as the backend stores them. */
function run(first: string, ...blocks: Block[]): Block[] {
  return blocks.map((block) => ({ ...block, run: first }) as Block)
}

describe('the steps the transcript shows as one row', () => {
  test('thinking without text shows only while it is what the running turn does now', () => {
    expect(rows(groupWork([...run('t1', thinking('t1'), call('a'), thinking('t2'), call('b')), text('r')], false)))
      .toEqual([['a', 'b'], 'r'])
    expect(rows(groupWork([...run('a', call('a'), call('b'), thinking('t3')), text('r')], false)))
      .toEqual([['a', 'b'], 'r'])
    expect(rows(groupWork([...run('t1', thinking('t1')), text('r')], false))).toEqual(['r'])
    expect(rows(groupWork([text('r'), ...run('a', call('a'), thinking('t2'))], true))).toEqual(['r', ['a'], 't2'])
  })

  test('each run the backend stored is a row of its own, even with nothing shown between', () => {
    expect(rows(groupWork([...run('a', call('a'), call('b')), text('r1'), ...run('c', call('c'), call('d'))], false)))
      .toEqual([['a', 'b'], 'r1', ['c', 'd']])
    expect(rows(groupWork([...run('a', call('a')), ...run('b', call('b'))], false))).toEqual([['a'], ['b']])
  })

  test('once the run ended, a lone call stays the row it was, and a call with another step is a group', () => {
    // A group of a lone call shows as the call's own row.
    expect(rows(groupWork([...run('a', call('a')), text('r')], false))).toEqual([['a'], 'r'])
    expect(rows(groupWork([...run('t', thinking('t', 'Plan the fix.'), call('a')), text('r')], false))).toEqual([['t', 'a'], 'r'])
  })

})

// Only a step that has ended joins its run, a call once it returned and
// thinking once it is written. At a running turn's end, what still runs is
// a row of its own under the run, so the run is what has happened. Before,
// the newest step was the run's own face, and the run opened it with the
// rest as one stack (the user's report: Ran 2 commands over the newest
// command opened together).
describe('the end of a running turn', () => {
  const executing = (id: string) => call(id, 'shell', { script: 'bun test', description: `Run ${id}` }, 'executing')

  test('what runs is a row of its own under the steps that ended', () => {
    // Thinking being written, a call waiting for its result, and a call being
    // written each stand alone.
    expect(rows(groupWork([text('r'), ...run('t1', thinking('t1'))], true))).toEqual(['r', 't1'])
    expect(rows(groupWork([text('r'), ...run('a', call('a'), executing('b'))], true))).toEqual(['r', ['a'], 'b'])
    expect(rows(groupWork([text('r'), ...run('a', call('a'), call('b')), writing('c')], true))).toEqual(['r', ['a', 'b'], 'pending-call:c-use'])
    // A call that ended joins the run even when one before it, started with it,
    // still runs.
    expect(rows(groupWork([text('r'), ...run('a', executing('a'), call('b'))], true))).toEqual(['r', ['b'], 'a'])
    // A call being written after what ends a run starts one.
    expect(rows(groupWork([...run('a', call('a')), text('r'), writing('b')], true))).toEqual([['a'], 'r', 'pending-call:b-use'])
    // Once the turn ended, nothing runs: every step is the run's.
    expect(rows(groupWork([text('r'), ...run('a', call('a'), thinking('t', 'Done.'))], false))).toEqual(['r', ['a', 't']])
  })

  test('a call whose command goes on in the background has ended, and wears the time of that command', () => {
    /**
     * A call that returned while its command ran on, as its stored view says.
     */
    const runsOn = (id: string): Block => ({
      ...call(id, 'shell', { script: 'sleep 600', description: `Run ${id}` }),
      view: { kind: 'shell', status: 'running', commandId: '7', runningMs: 15_000, idleMs: 0, viewTruncated: false, chunks: [] },
    } as Block)
    const live = groupWork([text('r'), ...run('a', call('a'), runsOn('s'))], true)
    expect(rows(live)).toEqual(['r', ['a', 's']])
    expect(runningOn((live[1] as WorkGroupBlock).steps).map((step) => step.id)).toEqual(['s'])
    expect(runningOn(steps(call('a'), runsOn('s'), call('b'), runsOn('w'), call('g', 'browse', { description: 'Open' }))).map((step) => step.id))
      .toEqual(['s', 'w'])
  })

  test('a run that holds only thinking shows once a step that says something joins it', () => {
    expect(rows(groupWork([text('r'), ...run('t', thinking('t', 'Plan the fix.'), executing('a'))], true))).toEqual(['r', 'a'])
    expect(rows(groupWork([text('r'), ...run('t', thinking('t', 'Plan the fix.'), call('a'))], true))).toEqual(['r', ['t', 'a']])
    // Ended, a lone thinking stays its own row.
    expect(rows(groupWork([text('r'), ...run('t', thinking('t', 'Plan the fix.')), text('d')], true))).toEqual(['r', ['t'], 'd'])
  })
})

// Thinking blocks next to each other show as one step. One Claude response
// thought twice in a row, 4 s then 1 s, and ran a shell call; the opened run
// showed Thought for 4s and Thought for 1s over the call.
describe('thinking blocks next to each other', () => {
  const at = (seconds: number) => new Date(Date.parse(createdAt) + seconds * 1000).toISOString()
  const thought = (id: string, text: string, second: number): Block => ({ ...thinking(id, text), createdAt: at(second) })
  const opened = (blocks: (Block | MessageListBlock)[], running: boolean) =>
    groupWork([text('q'), ...blocks], running).find((row) => row.type === 'work_group') as WorkGroupBlock

  test('join into one step whose time and text are theirs together', () => {
    const shell = { ...call('c'), createdAt: at(5) } as Block
    const group = opened([...run('k1', thought('k1', 'Read the test.', 0), thought('k2', 'Then run it.', 4), shell), { ...text('r'), createdAt: at(9) }], false)
    expect(group.steps.map((step) => step.type)).toEqual(['thinking', 'tool_call'])
    const [joined, next] = group.steps
    // Its time runs from the first block's start to the call's: 4 s and 1 s.
    expect(joined).toMatchObject({ id: 'k1', createdAt: at(0), text: 'Read the test.\n\nThen run it.' })
    expect(next!.type === 'tool_call' && next.createdAt).toBe(at(5))
    expect(workSummary(group.steps)).toBe('Ran 1 command')
  })

  test('around a call stay two steps', () => {
    const group = opened([...run('k1', thought('k1', 'Read the test.', 0), call('c'), thought('k2', 'It passes.', 4), call('d')), text('r')], false)
    expect(group.steps.map((step) => step.id)).toEqual(['k1', 'c', 'k2', 'd'])
  })

  test('alone, are one row, and a second one streaming keeps the row the first was', () => {
    expect(rows(groupWork([...run('k1', thought('k1', 'Read the test.', 0), thought('k2', 'Then run it.', 4)), text('r')], false)))
      .toEqual([['k1'], 'r'])
    // Being written, it is the row of its own of what runs, named as the first.
    const tail = (blocks: Block[]) => groupWork([text('q'), ...blocks], true).at(-1)!
    const first = tail(run('k1', thought('k1', 'Read the test.', 0)))
    const both = tail(run('k1', thought('k1', 'Read the test.', 0), thought('k2', '', 4)))
    expect(rowKey(both)).toBe(rowKey(first))
    expect(both).toMatchObject({ type: 'thinking', id: 'k1', text: 'Read the test.' })
  })
})

// A command's notice and an agent's receipt between two calls are steps of
// the work, as the backend stores them.
describe('the notices and receipts between the steps', () => {
  test('fold into the run, in their order, and the run reads what its calls did', () => {
    const steps = run('a', call('a'), call('b'), call('c'), notice('n', '17'), receipt('m', 'ag-2'), call('d'), call('e'))
    const grouped = groupWork([...steps, text('r')], false)
    expect(rows(grouped)).toEqual([['a', 'b', 'c', 'n', 'm', 'd', 'e'], 'r'])
    expect(workSummary((grouped[0] as WorkGroupBlock).steps)).toBe('Ran 5 commands')
  })

  test('the receipt of the user’s decision is no step: it shows between the runs', () => {
    expect(rows(groupWork([...run('a', call('a')), allowed('p'), ...run('b', call('b'))], false))).toEqual([['a'], 'p', ['b']])
  })

  test('a lone notice or receipt is its own row', () => {
    const end = receipt('m', 'ag-2', { type: 'completion', outcome: 'completed' })
    expect(rows(groupWork([text('r1'), ...run('n', notice('n', '17')), text('r2'), ...run('m', end), text('r3')], false)))
      .toEqual(['r1', ['n'], 'r2', ['m'], 'r3'])
  })

  test('a run without a call reads what its notices and receipts told, each kind once, in the order it first had it', () => {
    expect(workSummary(steps(notice('n1', '17'), notice('n2', '18')))).toBe('2 commands finished')
    expect(workSummary(steps(receipt('m1', 'ag-2'), receipt('m2', 'ag-3'), thinking('t', 'Both are halfway.')))).toBe('Heard from 2 agents')
    expect(workSummary(steps(
      notice('n1', '17', '18'),
      receipt('m1', 'ag-2', { type: 'completion', outcome: 'failed' }),
      receipt('m2', 'ag-3'),
      receipt('m3', 'ag-3'),
      notice('n2', '17'),
    ))).toBe('2 commands finished, 1 agent failed, heard from 1 agent')
    // A call says what the run did; its notices add nothing.
    expect(workSummary(steps(notice('n', '17'), check('s', 18)))).toBe('Checked 1 command')
  })

  test('a run without a call counts its commands and agents by how each ended', () => {
    expect(workSummary(steps(
      ended('n1', { kind: 'ended', exitCode: 2 }, '17'),
      ended('n2', { kind: 'stopped' }, '18'),
      notice('n3', '19'),
      receipt('m1', 'ag-2', { type: 'completion', outcome: 'completed' }),
      receipt('m2', 'ag-3', { type: 'completion', outcome: 'aborted' }),
    ))).toBe('1 command failed, 1 command stopped, 1 command finished, 1 agent finished, 1 agent stopped')
    // The first word is capitalized, as the row's start.
    expect(workSummary(steps(receipt('m', 'ag-2'), receipt('c', 'ag-2', { type: 'completion', outcome: 'failed' }))))
      .toBe('Heard from 1 agent, 1 agent failed')
  })
})

describe('what a group says it did', () => {
  const plan = thinking('t', 'See whether the build ended.')

  test('an ended group names each kind of call once, in the order the run first made it', () => {
    expect(workSummary(steps(plan, check('s', 17)))).toBe('Checked 1 command')
    // The design's example: command 17 checked twice.
    expect(workSummary(steps(plan, check('s1', 17), thinking('t2', 'Still building.'), check('s2', 17))))
      .toBe('Checked 1 command')
    expect(workSummary(steps(check('s1', 17, 18), check('s2', 18)))).toBe('Checked 2 commands')
    expect(workSummary(steps(call('a'), check('s', 17), call('b')))).toBe('Ran 2 commands, checked 1 command')
    expect(workSummary(steps(check('s1', 17), check('s2', 18), call('a')))).toBe('Checked 2 commands, ran 1 command')
  })

  test('a failed call, a stored call of a removed tool and a tool of another kind count by what they did', () => {
    expect(workSummary(steps(call('a', 'shell', { script: 'false' }, 'error'), call('r', 'read_file', { path: 'a.ts' }))))
      .toBe('Ran 1 command, used read_file')
    expect(workSummary(steps(plan, call('s', 'shell_status', { commandId: 17 })))).toBe('Used shell_status')
  })
})

// A run's row is named after the run the backend stored, so it stays one
// row whichever of its steps ends first; a step that runs is named after
// itself.
test('a run is one row under one name from its first step to its end', () => {
  const keys = (list: readonly MessageListBlock[]) => list.map((row) => (row.type === 'work_group' ? `${row.id}:${row.steps.length}` : rowKey(row)))
  const executing = (id: string) => call(id, 'shell', { script: 'bun test', description: `Run ${id}` }, 'executing')
  expect(keys(groupWork([text('t'), writing('c1')], true))).toEqual(['t', 'work:c1-use'])
  expect(keys(groupWork([text('t'), ...run('c1', executing('c1'))], true))).toEqual(['t', 'work:c1-use'])
  expect(keys(groupWork([text('t'), ...run('c1', call('c1'))], true))).toEqual(['t', 'run:c1:1'])
  expect(keys(groupWork([text('t'), ...run('c1', call('c1'), thinking('k', 'Next'))], true))).toEqual(['t', 'run:c1:1', 'work:k'])
  expect(keys(groupWork([text('t'), ...run('c1', call('c1')), writing('c2')], true))).toEqual(['t', 'run:c1:1', 'work:c2-use'])
  expect(keys(groupWork([text('t'), ...run('c1', call('c1'), call('c2'))], true))).toEqual(['t', 'run:c1:2'])
  expect(keys(groupWork([text('t'), ...run('c1', call('c1')), text('r')], false))).toEqual(['t', 'run:c1:1', 'r'])
  // A run that a thinking without text opened keeps that thinking's name once
  // it has gone.
  expect(keys(groupWork([text('t'), ...run('k', thinking('k'))], true))).toEqual(['t', 'work:k'])
  expect(keys(groupWork([text('t'), ...run('k', thinking('k'), call('c1'))], true))).toEqual(['t', 'run:k:1'])
  // A notice or a receipt that joins keeps the run's name, and one that starts
  // the run names it.
  expect(keys(groupWork([text('t'), ...run('c1', call('c1'), notice('n', '17'), receipt('m', 'ag-2'))], true))).toEqual(['t', 'run:c1:3'])
  expect(keys(groupWork([text('t'), ...run('n', notice('n', '17'))], true))).toEqual(['t', 'run:n:1'])
  expect(keys(groupWork([text('t'), ...run('n', notice('n', '17'), thinking('k'), call('c1'))], true))).toEqual(['t', 'run:n:2'])
})

// Review: parallel calls return in any order. Before, the run was named
// after its first ended step, so B returning before A named it after B and
// A's return renamed it after A: another row, closed again.
test('calls that return out of order keep their run one row under one name', () => {
  const executing = (id: string) => call(id, 'shell', { script: 'bun test', description: `Run ${id}` }, 'executing')
  const runRow = (blocks: Block[]) => groupWork([text('t'), ...blocks], true).find((row) => row.type === 'work_group') as WorkGroupBlock
  const before = runRow(run('a', executing('a'), call('b')))
  const after = runRow(run('a', call('a'), call('b')))
  expect(before.steps.map((step) => step.id)).toEqual(['b'])
  expect(after.steps.map((step) => step.id)).toEqual(['a', 'b'])
  expect(after.id).toBe(before.id)
})

// Review: a call waiting for its result is a row of its own wherever its run
// stands, not only in the run at the turn's end. Before, a call left
// executing in a run the agent's text ended folded into that run as if done.
test('a call that still runs in an earlier run is a row of its own', () => {
  const executing = (id: string) => call(id, 'shell', { script: 'bun test', description: `Run ${id}` }, 'executing')
  const list = groupWork([text('q'), ...run('k', thinking('k', 'Start both.'), executing('a')), text('r'), ...run('b', call('b'))], true)
  expect(rows(list)).toEqual(['q', 'a', 'r', ['b']])
})

// Review: two thinking blocks next to each other are one step, whose row
// follows its last part. Before, the row followed the first, which a second
// block had followed, so a second block streaming showed it ended.
test('a joined thinking step streams and ends with its last part', () => {
  const blocks = [text('q'), ...run('k1', thinking('k1', 'Read the test.'), thinking('k2', ''))]
  const row = groupWork(blocks, true).at(-1)!
  expect(row.id).toBe('k1')
  expect(isThinkingBlockStreaming(blocks as Block[], 'running', blocks.findIndex((block) => block.id === followedBlockId(row)))).toBe(true)
})
// A run says its total time in its sentence, running or ended, its steps'
// times added up: a call from its start until it returned, a command left
// in the background only until then, a thinking until the next block
// started, and never the model's wait between steps.
describe('the time a run took', () => {
  const at = (seconds: number) => new Date(Date.parse(createdAt) + seconds * 1000).toISOString()
  const timed = (block: Block, start: number, end?: number): Block =>
    ({ ...block, createdAt: at(start), ...(end === undefined ? {} : { endedAt: at(end) }) }) as Block
  const label = (blocks: (Block | MessageListBlock)[], running: boolean) =>
    runLabel(groupWork([text('q'), ...blocks], running).find((row) => row.type === 'work_group') as WorkGroupBlock)

  test('adds its steps’ times and leaves out the waits between them', () => {
    // Thinks 4 s, runs a command 10 s, the model is waited for 3 s, starts a
    // command that goes on in the background and returns after 2 s, which
    // later reports its end; the answer comes 30 s later.
    const steps = run('k',
      timed(thinking('k', 'Run the suite.'), 0),
      timed(call('a'), 4, 14),
      timed(call('b', 'shell', { script: 'bun dev', description: 'Start the server' }), 17, 19),
      timed(notice('n', '7'), 40))
    expect(label([...steps, timed(text('r'), 49)], false)).toBe('Ran 2 commands for 16s')
    // While the turn runs, the same steps say the same.
    expect(label(steps, true)).toBe('Ran 2 commands for 16s')
    // Thinking that ended before a call that still runs ends where that call
    // starts.
    expect(label(run('k', timed(thinking('k', 'Check.'), 0), timed(call('a'), 6, 8), timed(thinking('t', 'Then run it.'), 8), timed(call('c', 'shell', undefined, 'executing'), 70)), true))
      .toBe('Ran 1 command for 1m 10s')
    // Review: a thinking ends where the next block starts in the transcript,
    // a call that still runs included. Thinking 0–6 s, call A runs from 6 s,
    // call B 7–10 s returned first: 6 s and 3 s, never the thinking until B.
    expect(label(run('k', timed(thinking('k', 'Run both.'), 0), timed(call('a', 'shell', undefined, 'executing'), 6), timed(call('b'), 7, 10)), true))
      .toBe('Ran 1 command for 9s')
  })

  test('a run under a second says no time', () => {
    expect(label(run('a', timed(call('a'), 0, 0.3), timed(call('b'), 1, 1.2)), false)).toBe('Ran 2 commands')
  })
})

// A fixed bug of complexity: each step of a run looked for its block from the
// start of the transcript, so grouping read the transcript's ids as many times
// as it had steps, two million reads for 2,000 blocks, at every change of a
// streaming turn. The reads are counted, not timed.
test('grouping a long transcript reads each block\'s id a few times, however long the transcript is', () => {
  let reads = 0
  const counted = (block: Block): Block => {
    const { id, ...rest } = block
    return Object.defineProperty(rest, 'id', { enumerable: true, get: () => (reads += 1, id) }) as Block
  }
  const transcript = Array.from({ length: 400 }, (_, turn) => [
    ...run(`a${turn}`, call(`a${turn}`), call(`b${turn}`), call(`c${turn}`), call(`d${turn}`), call(`e${turn}`)),
    text(`r${turn}`),
  ]).flat().map(counted)
  const grouped = groupWork(transcript, false)
  expect(grouped).toHaveLength(800)
  expect(reads).toBeLessThan(4 * transcript.length)
})

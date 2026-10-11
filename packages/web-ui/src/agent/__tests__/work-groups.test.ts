import { describe, expect, test } from 'bun:test'
import type { Block } from '@demicodes/protocol'
import type { MessageListBlock } from '../pending-steers'
import { groupWork, workRunning, workSummary, type WorkGroupBlock } from '../work-groups'
import { createdAt, model } from './agent-harness'

// Cost: pure, a few milliseconds for the file.

const thinking = (id: string, text = ''): Block => ({ type: 'thinking', id, createdAt, model, text, signature: null })
const text = (id: string): Block => ({ type: 'text', id, createdAt, model, text: 'Done.' })
const call = (
  id: string,
  toolName = 'shell',
  input: Record<string, unknown> = { script: 'true', description: `Run ${id}` },
  status: 'completed' | 'error' = 'completed',
): Block => ({
  type: 'tool_call',
  id,
  createdAt,
  model,
  toolUseId: `${id}-use`,
  toolName,
  status,
  input: JSON.stringify(input),
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
/** A command's notice: the end of each command it names, each as `event` says it ended. */
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
    expect(rows(groupWork([text('r'), ...run('a', call('a'), thinking('t2'))], true))).toEqual(['r', ['a', 't2']])
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

  test('the end of a running turn is always a group, from its first step, the call being written included', () => {
    const live = groupWork([text('r'), ...run('t1', thinking('t1'))], true)
    expect(rows(live)).toEqual(['r', ['t1']])
    expect((live[1] as WorkGroupBlock).live).toBe(true)
    // The empty thinking that opened the run gives the group its id, so the
    // row stays the same row as the calls roll over it.
    const later = groupWork([text('r'), ...run('t1', thinking('t1'), call('a')), writing('b')], true)
    expect(rows(later)).toEqual(['r', ['a', 'pending-call:b-use']])
    expect(later[1]!.id).toBe(live[1]!.id)
    // A call being written after what ends a run starts one.
    expect(rows(groupWork([...run('a', call('a')), text('r'), writing('b')], true))).toEqual([['a'], 'r', ['pending-call:b-use']])
  })
})

// A command's notice and an agent's receipt between two calls are steps of
// the work, as the backend stores them (`runtime.md` § Work groups).
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
    expect(workRunning(steps(notice('n', '17'), receipt('m', 'ag-2')))).toBe('1 command finished, heard from 1 agent')
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

  test('a running group counts the calls being written, and names what it does when it runs no command', () => {
    expect(workRunning(steps(call('a'), check('s', 17), call('c'), writing('d')))).toBe('Running 3 commands')
    expect(workRunning(steps(plan, check('s', 17)))).toBe('Checking 1 command')
    expect(workRunning(steps(plan, check('s', 17), writing('r', 'read_file')))).toBe('Checking 1 command, using read_file')
    expect(workRunning(steps(plan))).toBe('Thinking')
  })
})

// The run at the end of a running turn is one row from its first step: a
// step that joins never adds a row that then merges into it, and the row
// keeps its name as a call being written becomes its block and as the run
// ends, so the list keeps the row it measured (`runtime.md` § Work groups).
test('a run is one row under one name from its first step to its end', () => {
  const ids = (list: readonly MessageListBlock[]) => list.map((row) => (row.type === 'work_group' ? `${row.id}:${row.steps.length}` : row.id))
  expect(ids(groupWork([text('t'), writing('c1')], true))).toEqual(['t', 'work:c1-use:1'])
  expect(ids(groupWork([text('t'), ...run('c1', call('c1'))], true))).toEqual(['t', 'work:c1-use:1'])
  expect(ids(groupWork([text('t'), ...run('c1', call('c1'), thinking('k', 'Next'))], true))).toEqual(['t', 'work:c1-use:2'])
  expect(ids(groupWork([text('t'), ...run('c1', call('c1')), writing('c2')], true))).toEqual(['t', 'work:c1-use:2'])
  expect(ids(groupWork([text('t'), ...run('c1', call('c1')), text('r')], false))).toEqual(['t', 'work:c1-use:1', 'r'])
  // A thinking first names the row, also once it has nothing to show and goes.
  expect(ids(groupWork([text('t'), ...run('k', thinking('k'))], true))).toEqual(['t', 'work:k:1'])
  expect(ids(groupWork([text('t'), ...run('k', thinking('k'), call('c1'))], true))).toEqual(['t', 'work:k:1'])
  expect(ids(groupWork([text('t'), ...run('k', thinking('k'), call('c1')), text('r')], true))).toEqual(['t', 'work:k:1', 'r'])
  // A notice or a receipt that joins keeps the run's name, and one that starts the run names it.
  expect(ids(groupWork([text('t'), ...run('c1', call('c1'), notice('n', '17'), receipt('m', 'ag-2'))], true))).toEqual(['t', 'work:c1-use:3'])
  expect(ids(groupWork([text('t'), ...run('n', notice('n', '17'))], true))).toEqual(['t', 'work:n:1'])
  expect(ids(groupWork([text('t'), ...run('n', notice('n', '17'), thinking('k'), call('c1'))], true))).toEqual(['t', 'work:n:2'])
})

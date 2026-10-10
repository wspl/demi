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
  call: { toolUseId: `${id}-use`, toolName, description: null },
})
const steps = (...blocks: (Block | MessageListBlock)[]) => blocks as WorkGroupBlock['steps']

/** Each row as its id, a group as the ids of its steps. */
function rows(list: readonly MessageListBlock[]): (string | string[])[] {
  return list.map((row) => (row.type === 'work_group' ? row.steps.map((step) => step.id) : row.id))
}

describe('the steps the transcript shows as one row', () => {
  test('thinking without text shows only while it is what the running turn does now', () => {
    expect(rows(groupWork([thinking('t1'), call('a'), thinking('t2'), call('b'), text('r')], false)))
      .toEqual([['a', 'b'], 'r'])
    expect(rows(groupWork([call('a'), call('b'), thinking('t3'), text('r')], false)))
      .toEqual([['a', 'b'], 'r'])
    expect(rows(groupWork([thinking('t1'), text('r')], false))).toEqual(['r'])
    expect(rows(groupWork([text('r'), call('a'), thinking('t2')], true))).toEqual(['r', ['a', 't2']])
  })

  test('a reply ends a run, and the next run is a group of its own', () => {
    expect(rows(groupWork([call('a'), call('b'), text('r1'), call('c'), call('d')], false)))
      .toEqual([['a', 'b'], 'r1', ['c', 'd']])
  })

  test('once the run ended, a lone call is its own row and a call with another step is a group', () => {
    expect(rows(groupWork([call('a'), text('r')], false))).toEqual(['a', 'r'])
    expect(rows(groupWork([thinking('t', 'Plan the fix.'), call('a'), text('r')], false))).toEqual([['t', 'a'], 'r'])
  })

  test('the end of a running turn is always a group, from its first step, the call being written included', () => {
    const live = groupWork([text('r'), thinking('t1')], true)
    expect(rows(live)).toEqual(['r', ['t1']])
    expect((live[1] as WorkGroupBlock).live).toBe(true)
    // The empty thinking that opened the run gives the group its id, so the
    // row stays the same row as the calls roll over it.
    const later = groupWork([text('r'), thinking('t1'), call('a'), writing('b')], true)
    expect(rows(later)).toEqual(['r', ['a', 'pending-call:b-use']])
    expect(later[1]!.id).toBe(live[1]!.id)
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

import { describe, expect, test } from 'bun:test'
import type { Block } from '@demicodes/protocol'
import type { MessageListBlock } from '../pending-steers'
import { groupWork, workRunning, workSummary, type WorkGroupBlock } from '../work-groups'
import { createdAt, model } from './agent-harness'

// Cost: pure, a few milliseconds for the file.

const thinking = (id: string, text = ''): Block => ({ type: 'thinking', id, createdAt, model, text, signature: null })
const text = (id: string): Block => ({ type: 'text', id, createdAt, model, text: 'Done.' })
const call = (id: string, toolName = 'shell_exec'): Block => ({
  type: 'tool_call',
  id,
  createdAt,
  model,
  toolUseId: `${id}-use`,
  toolName,
  status: 'completed',
  input: JSON.stringify({ script: 'true', description: `Run ${id}` }),
  output: [],
  view: null,
})
const writing = (id: string): MessageListBlock => ({
  type: 'pending_call',
  id: `pending-call:${id}-use`,
  call: { toolUseId: `${id}-use`, toolName: 'shell_exec', description: null },
})

/** Each row as its id, a group as the ids of its steps. */
function rows(list: readonly MessageListBlock[]): (string | string[])[] {
  return list.map((row) => (row.type === 'work_group' ? row.steps.map((step) => step.id) : row.id))
}

describe('the steps the transcript shows as one row', () => {
  test('thinking without text is covered by the step after it, and stays only before a reply', () => {
    expect(rows(groupWork([thinking('t1'), call('a'), thinking('t2'), call('b'), text('r')], false)))
      .toEqual([['a', 'b'], 'r'])
    expect(rows(groupWork([call('a'), call('b'), thinking('t3'), text('r')], false)))
      .toEqual([['a', 'b', 't3'], 'r'])
    expect(rows(groupWork([thinking('t1'), text('r')], false))).toEqual([['t1'], 'r'])
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

  test('a group says what it ran, while it runs and once it ended', () => {
    const steps = [call('a'), call('b', 'shell_status'), call('c')] as WorkGroupBlock['steps']
    expect(workSummary(steps)).toBe('Ran 2 commands')
    expect(workRunning([...steps, writing('d')] as WorkGroupBlock['steps'])).toBe('Running 3 commands')
    expect(workSummary([call('w', 'yield')] as WorkGroupBlock['steps'])).toBe('1 step')
  })
})

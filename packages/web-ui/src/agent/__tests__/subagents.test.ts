import { expect, test } from 'bun:test'
import { EMPTY_TRANSCRIPT } from '@demicodes/conversation-client'
import {
  agentsChip,
  firstInspectSubagentId,
  formatDuration,
  formatSubagentDuration,
  finishedSubagents,
  runningSubagents,
  subagentPanelTabs,
  subagentStatus,
  type SubagentRecord,
} from '../subagents'

function agent(
  partial: Pick<SubagentRecord, 'id' | 'name' | 'phase'> & Partial<SubagentRecord>,
): SubagentRecord {
  return {
    startedAt: '2026-09-09T00:00:00.000Z',
    history: EMPTY_TRANSCRIPT,
    pendingCalls: [],
    failures: {},
    ...partial,
  }
}

test('running agents are oldest first; finished are newest close first', () => {
  const agents = [
    agent({
      id: 'a',
      name: 'A',
      phase: 'running',
      startedAt: '2026-09-09T00:02:00.000Z',
    }),
    agent({
      id: 'b',
      name: 'B',
      phase: 'completed',
      startedAt: '2026-09-09T00:00:00.000Z',
      endedAt: '2026-09-09T00:03:00.000Z',
    }),
    agent({
      id: 'c',
      name: 'C',
      phase: 'running',
      startedAt: '2026-09-09T00:01:00.000Z',
    }),
    agent({
      id: 'd',
      name: 'D',
      phase: 'aborted',
      startedAt: '2026-09-09T00:00:00.000Z',
      endedAt: '2026-09-09T00:04:00.000Z',
    }),
  ]
  expect(runningSubagents(agents).map((item) => item.id)).toEqual(['c', 'a'])
  expect(finishedSubagents(agents).map((item) => item.id)).toEqual(['d', 'b'])
})

test('status follows the child phase', () => {
  expect(subagentStatus('running')).toBe('active')
  expect(subagentStatus('completed')).toBe('done')
  expect(subagentStatus('aborted')).toBe('aborted')
  expect(subagentStatus('error')).toBe('error')
})

test('duration is compact and uses endedAt once the child is finished', () => {
  expect(formatDuration(12_400)).toBe('12s')
  expect(formatDuration(60_000)).toBe('1m')
  expect(formatDuration(134_000)).toBe('2m14s')
  expect(formatDuration(3_780_000)).toBe('1h3m')
  const running = agent({
    id: 'r',
    name: 'R',
    phase: 'running',
    startedAt: '2026-09-09T00:00:00.000Z',
  })
  expect(
    formatSubagentDuration(running, Date.parse('2026-09-09T00:02:14.000Z')),
  ).toBe('2m14s')
  const done = agent({
    id: 'd',
    name: 'D',
    phase: 'completed',
    startedAt: '2026-09-09T00:00:00.000Z',
    endedAt: '2026-09-09T00:03:00.000Z',
  })
  expect(formatSubagentDuration(done, Date.parse('2026-09-09T01:00:00.000Z'))).toBe(
    '3m',
  )
})

test('a running inspect lists every running child; a finished inspect sits beside them', () => {
  const agents = [
    agent({
      id: 'run-old',
      name: 'Old',
      phase: 'running',
      startedAt: '2026-09-09T00:01:00.000Z',
    }),
    agent({
      id: 'run-new',
      name: 'New',
      phase: 'running',
      startedAt: '2026-09-09T00:02:00.000Z',
    }),
    agent({
      id: 'done',
      name: 'Done',
      phase: 'completed',
      endedAt: '2026-09-09T00:03:00.000Z',
    }),
    agent({
      id: 'aborted',
      name: 'Aborted',
      phase: 'aborted',
      endedAt: '2026-09-09T00:04:00.000Z',
    }),
  ]
  expect(subagentPanelTabs(agents, null)).toEqual([])
  expect(subagentPanelTabs(agents, 'missing')).toEqual([])
  expect(subagentPanelTabs(agents, 'run-new').map((item) => item.id)).toEqual([
    'run-old',
    'run-new',
  ])
  expect(subagentPanelTabs(agents, 'done').map((item) => item.id)).toEqual([
    'run-old',
    'run-new',
    'done',
  ])
  expect(subagentPanelTabs(agents, 'aborted').map((item) => item.id)).toEqual([
    'run-old',
    'run-new',
    'aborted',
  ])
  expect(subagentPanelTabs([agents[2]!], 'done').map((item) => item.id)).toEqual([
    'done',
  ])
})

test('inspect opens the oldest running child, or the newest finished child', () => {
  const agents = [
    agent({
      id: 'run-new',
      name: 'New',
      phase: 'running',
      startedAt: '2026-09-09T00:02:00.000Z',
    }),
    agent({
      id: 'run-old',
      name: 'Old',
      phase: 'running',
      startedAt: '2026-09-09T00:01:00.000Z',
    }),
    agent({
      id: 'done',
      name: 'Done',
      phase: 'completed',
      endedAt: '2026-09-09T00:03:00.000Z',
    }),
  ]
  expect(firstInspectSubagentId(agents)).toBe('run-old')
  expect(
    firstInspectSubagentId(agents.filter((item) => item.phase !== 'running')),
  ).toBe('done')
  expect(firstInspectSubagentId([])).toBeNull()
})

test('the dock chip counts live children, reads Agents once none runs, and hides without children', () => {
  const running = (id: string) => agent({ id, name: id, phase: 'running' })
  const completed = agent({ id: 'done', name: 'Done', phase: 'completed' })
  const aborted = agent({ id: 'stopped', name: 'Stopped', phase: 'aborted' })
  const failed = agent({ id: 'failed', name: 'Failed', phase: 'error' })
  // Two live children: the finished ones beside them never count.
  expect(agentsChip([running('a'), completed, running('b'), aborted, failed])).toEqual({
    label: '2 Agents',
    running: true,
  })
  expect(agentsChip([running('a'), completed])).toEqual({ label: '1 Agent', running: true })
  // Every child has finished: the chip stays, with no count, for their history.
  expect(agentsChip([completed, aborted, failed])).toEqual({ label: 'Agents', running: false })
  // The conversation never had a child.
  expect(agentsChip([])).toBeNull()
})

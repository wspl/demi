import { expect, test } from 'bun:test'
import { EMPTY_TRANSCRIPT } from '@demicodes/conversation-client'
import {
  agentsChipLabel,
  childBrief,
  formatSubagentDuration,
  runEffortWords,
  finishedSubagents,
  runningSubagents,
  subagentPanelTabs,
  subagentStatus,
  type SubagentRecord,
} from '../subagents'
import { useSessionPanels } from '../useSessionPanels'

function agent(
  partial: Pick<SubagentRecord, 'id' | 'name' | 'phase'> & Partial<SubagentRecord>,
): SubagentRecord {
  return {
    number: 1,
    profile: null,
    model: { providerId: 'anthropic', provider: 'Anthropic', model: 'Claude Opus 5.5', effort: 'high', fast: false },
    round: 1,
    forkedFrom: null,
    startedAt: '2026-09-09T00:00:00.000Z',
    history: EMPTY_TRANSCRIPT,
    sessionPhase: 'idle',
    pendingCalls: [],
    waitingSince: null,
    failures: {},
    contextUsage: null,
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

test('a child\'s time is in the reset times\' duration words and ends with the child', () => {
  const running = agent({
    id: 'r',
    name: 'R',
    phase: 'running',
    startedAt: '2026-09-09T00:00:00.000Z',
  })
  expect(
    formatSubagentDuration(running, Date.parse('2026-09-09T00:02:14.000Z')),
  ).toBe('2m 14s')
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

test.each([
  { scenario: 'a level', effort: 'medium', words: 'Medium' },
  { scenario: 'thinking turned off', effort: 'off', words: 'Thinking off' },
  { scenario: 'a model without a thinking setting', effort: null, words: null },
])('a child on $scenario names its effort as the model menu does', ({ effort, words }) => {
  const model = { providerId: 'anthropic', provider: 'Anthropic', model: 'Sonnet 5.5', effort, fast: false }
  expect(runEffortWords(model)).toBe(words)
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
  // None picked: the running children alone.
  expect(subagentPanelTabs(agents, null).map((item) => item.id)).toEqual(['run-old', 'run-new'])
  expect(subagentPanelTabs(agents, 'missing').map((item) => item.id)).toEqual(['run-old', 'run-new'])
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

test('the Agents panel opens on the oldest running child, or empty when none runs', () => {
  const running = [
    agent({ id: 'run-new', name: 'New', phase: 'running', startedAt: '2026-09-09T00:02:00.000Z' }),
    agent({ id: 'run-old', name: 'Old', phase: 'running', startedAt: '2026-09-09T00:01:00.000Z' }),
  ]
  const done = agent({ id: 'done', name: 'Done', phase: 'completed', endedAt: '2026-09-09T00:03:00.000Z' })
  let agents: SubagentRecord[] = [...running, done]
  const panels = useSessionPanels(() => agents, () => [])
  panels.toggleAgents()
  expect([panels.agentsOpen.value, panels.activeSubagentId.value]).toEqual([true, 'run-old'])
  panels.toggleAgents()
  expect(panels.agentsOpen.value).toBe(false)
  // Only finished children: it opens on none of them, never one the user did not pick.
  agents = [done]
  panels.toggleAgents()
  expect([panels.agentsOpen.value, panels.activeSubagentId.value]).toEqual([true, null])
})

test('the dock chip counts live children and shows only while one runs', () => {
  const running = (id: string) => agent({ id, name: id, phase: 'running' })
  const completed = agent({ id: 'done', name: 'Done', phase: 'completed' })
  const aborted = agent({ id: 'stopped', name: 'Stopped', phase: 'aborted' })
  const failed = agent({ id: 'failed', name: 'Failed', phase: 'error' })
  // Two live children: the finished ones beside them never count.
  expect(agentsChipLabel([running('a'), completed, running('b'), aborted, failed])).toBe('2 Agents')
  expect(agentsChipLabel([running('a'), completed])).toBe('1 Agent')
  // Every child has finished: no chip; their receipts and links open them.
  expect(agentsChipLabel([completed, aborted, failed])).toBeNull()
  // The conversation never had a child.
  expect(agentsChipLabel([])).toBeNull()
})

test('a child\'s brief renders as an agent\'s Markdown, and the conversation\'s own messages keep the user dialect', () => {
  const brief = '# Research: login flake\n\n- Read the test\n- Find the cause'
  // In a child's transcript: the brief its spawn wrote, as Markdown.
  expect(childBrief('child-1', [{ type: 'text', text: brief }])).toBe(brief)
  // The same text the user typed in the conversation's own transcript.
  expect(childBrief(null, [{ type: 'text', text: brief }])).toBeNull()
  // A message that holds a file is the user's, wherever it stands.
  expect(childBrief('child-1', [
    { type: 'text', text: 'See ' },
    { type: 'attachment', name: 'a.png', path: '/h/a.png', mediaType: 'image/png', sizeBytes: 4, sha256: 'a'.repeat(64) },
  ])).toBeNull()
})

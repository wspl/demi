import { expect, test } from 'bun:test'
import { EMPTY_TRANSCRIPT } from '@demicodes/conversation-client'
import { agentLinkWindow, findAgent, linkedAgent } from '../agent-links'
import type { SubagentRecord } from '../subagents'

// A message's link to an agent. Pure,
// under a millisecond.

function agent(id: string, number: number, phase: SubagentRecord['phase']): SubagentRecord {
  return {
    id, number, name: `Agent ${id}`, onPath: true, phase, sessionPhase: 'idle', startedAt: '2026-09-09T00:00:00.000Z',
    profile: null, model: { providerId: 'anthropic', provider: 'Anthropic', model: 'Claude Opus 5.5', effort: 'high', fast: false }, round: 1, forkedFrom: null,
    history: EMPTY_TRANSCRIPT, pendingCalls: [], waitingSince: null, failures: {}, contextUsage: null, cwd: '/work',
  }
}

const agents = [agent('ag-icons', 4, 'running'), agent('ag-cjk', 5, 'completed')]

test('a link names an agent by the number spawn printed; the root is 0', () => {
  expect(['agent:4', 'agent:0', ' agent:12 ', 'agent:04', 'agent:', 'agent:x', 'agents:4', 'agent:4#x'].map(linkedAgent))
    .toEqual([4, 0, 12, null, null, null, null, null])
  expect(findAgent(agents, 5)).toEqual({ number: 5, description: 'Agent ag-cjk', run: agents[1] })
  expect(findAgent(agents, 0)).toMatchObject({ number: 0, run: null })
  expect(findAgent(agents, 9)).toBeNull()
})

test('a click opens the Agents window on the agent’s tab, and on the tab it shows leaves it open', () => {
  const closed = { open: false, activeId: null }
  const onFinished = agentLinkWindow(closed, agents, 5)
  expect(onFinished).toEqual({ open: true, activeId: 'ag-cjk' })
  expect(agentLinkWindow(onFinished!, agents, 5)).toEqual({ open: true, activeId: 'ag-cjk' })
  // Another agent's link moves the open window to its tab.
  expect(agentLinkWindow(onFinished!, agents, 4)).toEqual({ open: true, activeId: 'ag-icons' })
  // The main agent's link brings the conversation's transcript into view.
  expect(agentLinkWindow(onFinished!, agents, 0)).toEqual({ open: false, activeId: 'ag-cjk' })
  // A number the conversation does not have changes nothing.
  expect(agentLinkWindow(onFinished!, agents, 9)).toBeNull()
})

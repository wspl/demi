import { expect, test } from 'bun:test'
import type { UserContentBlock } from '@demi/core'
import { useAgentInputActions } from '../message-input/useAgentInputActions'
import type { AgentWorkspace } from '../workspace'

test('input submit sends a new turn while idle', async () => {
  const calls: string[] = []
  const workspace = fakeWorkspace('idle', calls)
  const actions = useAgentInputActions({
    workspace,
    conversationId: 'conversation-1',
    buildSubmitPayload: () => [{ type: 'text', text: 'new turn' }],
    clearInput: () => calls.push('clear'),
  })

  await actions.handleSubmit()

  expect(calls).toEqual(['clear', 'send:new turn'])
})

test('input submit steers the active turn while running', async () => {
  const calls: string[] = []
  const workspace = fakeWorkspace('running', calls)
  const actions = useAgentInputActions({
    workspace,
    conversationId: 'conversation-1',
    buildSubmitPayload: () => [{ type: 'text', text: 'refine this turn' }],
    clearInput: () => calls.push('clear'),
  })

  await actions.handleSubmit()

  expect(calls).toEqual(['clear', 'steer:refine this turn'])
})

function fakeWorkspace(phase: 'idle' | 'running' | 'compacting', calls: string[]): AgentWorkspace {
  return {
    sessions: {
      'conversation-1': { phase },
    },
    send: async (_id: string, content: UserContentBlock[]) => {
      calls.push(`send:${textContent(content)}`)
    },
    steer: async (_id: string, content: UserContentBlock[]) => {
      calls.push(`steer:${textContent(content)}`)
    },
    setModel: () => {},
    abort: async () => {},
    compact: async () => {},
  } as unknown as AgentWorkspace
}

function textContent(content: UserContentBlock[]): string {
  return content.map((block) => (block.type === 'text' ? block.text : `[${block.type}]`)).join('\n')
}

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

test('input submit queues a new turn while running', async () => {
  const calls: string[] = []
  const workspace = fakeWorkspace('running', calls)
  const actions = useAgentInputActions({
    workspace,
    conversationId: 'conversation-1',
    buildSubmitPayload: () => [{ type: 'text', text: 'run after this' }],
    clearInput: () => calls.push('clear'),
  })

  await actions.handleSubmit()

  expect(calls).toEqual(['clear', 'send:run after this'])
})

test('input steer action steers the active turn while running', async () => {
  const calls: string[] = []
  const workspace = fakeWorkspace('running', calls)
  const actions = useAgentInputActions({
    workspace,
    conversationId: 'conversation-1',
    buildSubmitPayload: () => [{ type: 'text', text: 'refine this turn' }],
    clearInput: () => calls.push('clear'),
  })

  await actions.handleSteerSubmit()

  expect(calls).toEqual(['clear', 'steer:refine this turn'])
})

test('input queue action sends a new turn while running', async () => {
  const calls: string[] = []
  const workspace = fakeWorkspace('running', calls)
  const actions = useAgentInputActions({
    workspace,
    conversationId: 'conversation-1',
    buildSubmitPayload: () => [{ type: 'text', text: 'run after this' }],
    clearInput: () => calls.push('clear'),
  })

  await actions.handleQueueSubmit()

  expect(calls).toEqual(['clear', 'send:run after this'])
})

test('input submit emits empty-submit when there is no payload', async () => {
  const calls: string[] = []
  const workspace = fakeWorkspace('running', calls)
  const actions = useAgentInputActions({
    workspace,
    conversationId: 'conversation-1',
    buildSubmitPayload: () => null,
    clearInput: () => calls.push('clear'),
    emitEmptySubmit: () => calls.push('empty-submit'),
  })

  await actions.handleSubmit()

  expect(calls).toEqual(['empty-submit'])
})

test('empty input submit sends the last queued message when queue exists', async () => {
  const calls: string[] = []
  const workspace = fakeWorkspace('running', calls, [
    { id: 'queued-first' },
    { id: 'queued-last' },
  ])
  const actions = useAgentInputActions({
    workspace,
    conversationId: 'conversation-1',
    buildSubmitPayload: () => null,
    clearInput: () => calls.push('clear'),
    emitEmptySubmit: () => calls.push('empty-submit'),
  })

  await actions.handleSubmit()

  expect(calls).toEqual(['send-queued:queued-last'])
})

function fakeWorkspace(
  phase: 'idle' | 'running' | 'compacting',
  calls: string[],
  queue: Array<{ id: string }> = [],
): AgentWorkspace {
  return {
    sessions: {
      'conversation-1': { phase, queue },
    },
    send: async (_id: string, content: UserContentBlock[]) => {
      calls.push(`send:${textContent(content)}`)
    },
    sendQueuedMessage: (_id: string, messageId: string) => {
      calls.push(`send-queued:${messageId}`)
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

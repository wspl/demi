import { expect, test } from 'bun:test'
import type { Block } from '@demicodes/core'
import { storedRunningCommandIds } from '../server/summaries'

function shellCall(id: string, commandId: string, status: 'running' | 'exited'): Block {
  return {
    type: 'tool_call',
    id,
    turnId: 'turn-1',
    createdAt: '2026-09-12T00:00:00.000Z',
    toolUseId: id,
    toolName: 'shell_exec',
    input: '{}',
    view: { kind: 'shell', status, shellId: 'shell-1', commandId },
  } as unknown as Block
}

test('a stored transcript names the commands it last saw running', () => {
  const blocks = [
    shellCall('a', 'cmd-1', 'running'),
    shellCall('b', 'cmd-2', 'running'),
    shellCall('c', 'cmd-1', 'exited'),
    { type: 'text', id: 't', turnId: 'turn-1', createdAt: '', content: '' } as unknown as Block,
  ]
  expect(storedRunningCommandIds(blocks)).toEqual(['cmd-2'])
})

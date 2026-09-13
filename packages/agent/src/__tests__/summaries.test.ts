import { expect, test } from 'bun:test'
import type { Block } from '@demicodes/core'
import type { ShellToolView } from '../tools'
import { shellToolViewSchema } from '../client/shell-view'
import { storedRunningCommandIds } from '../server/summaries'

function shellView(
  commandId: string,
  status: ShellToolView['status']
): ShellToolView {
  return {
    kind: 'shell',
    status,
    shellId: 'shell-1',
    commandId,
    runningMs: 12,
    idleMs: 0,
    chunks: [{ stream: 'stdout', text: 'out' }],
    viewTruncated: false,
  }
}

function shellCall(
  id: string,
  commandId: string,
  status: ShellToolView['status']
): Block {
  return {
    type: 'tool_call',
    id,
    turnId: 'turn-1',
    createdAt: '2026-09-12T00:00:00.000Z',
    toolUseId: id,
    toolName: 'shell_exec',
    input: '{}',
    view: shellView(commandId, status),
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

test('the shell view schema refuses a view another tool wrote', () => {
  expect(shellToolViewSchema.safeParse(shellView('cmd-1', 'running')).success)
    .toBe(true)
  expect(shellToolViewSchema.safeParse({
    kind: 'repeated_shell_exec',
    script: 'ls',
    count: 7
  }).success).toBe(false)
  expect(shellToolViewSchema.safeParse({
    ...shellView('cmd-1', 'running'),
    chunks: [{ stream: 'stdout' }],
  }).success).toBe(false)
})

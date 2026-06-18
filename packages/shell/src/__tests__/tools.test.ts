import { access, mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { AgentSession, type AgentDefinition } from '@demi/base-agent'
import { StubProvider, events, type InferenceRequest } from '@demi/provider'
import { BashEnvironment, createShellSessionTools, toToolResult, type ShellToolResult } from '../index'
import { LocalHost } from '../local-host'

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

test('toToolResult stringifies shell results with BigInt metadata without throwing', () => {
  const result: ShellToolResult = {
    status: 'exited',
    sessionId: 'tool-bigint-session',
    exitCode: 0,
    output: {
      stdoutDelta: '',
      stderrDelta: '',
      stdoutTail: '',
      stderrTail: '',
      totalStdoutBytes: 0,
      totalStderrBytes: 0,
      truncated: false,
    },
    audit: [],
    commandMetadata: [
      {
        kind: 'registered-command',
        name: 'meta',
        args: [],
        metadata: { count: 42n },
      },
    ],
  }

  const toolResult = toToolResult(result)
  const first = toolResult.output[0]
  if (first?.type !== 'text') throw new Error('expected text result')

  expect(JSON.parse(first.text).commandMetadata[0].metadata.count).toBe('42')
  expect(toolResult.metadata).toBe(result)
})

test('createShellSessionTools integrates shell_exec and shell_wait with AgentSession', async () => {
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    sessionIdFactory: () => 'tool-session',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition: AgentDefinition<Record<string, never>> = {
    name: 'shell-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => createShellSessionTools(environment),
  }
  const provider = new StubProvider([
    [events.toolCall('call-1', 'shell_exec', { script: 'sh -c "sleep 0.02; printf done"', yieldAfterMs: 1 })],
    (request: InferenceRequest) => {
      const result = latestToolResult(request.items)
      expect(result.status).toBe('running')
      return [events.toolCall('call-2', 'shell_wait', { sessionId: result.sessionId, yieldAfterMs: 1_000 })]
    },
    (request: InferenceRequest) => {
      const result = latestToolResult(request.items)
      expect(result.status).toBe('exited')
      expect(result.output.stdoutDelta).toBe('done')
      return [events.text('finished'), events.response()]
    },
  ])
  const session = new AgentSession({
    provider,
    model,
    cwd: process.cwd(),
    definition,
  })

  await session.send([{ type: 'text', text: 'run command' }])

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'tool_call',
    'text',
    'response',
  ])
})

test('shell_exec observes AgentSession abort signals and terminates the foreground process', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-shell-tool-abort-'))
  const leakedPath = join(root, 'tool-abort-leaked.txt')
  const environment = new BashEnvironment({
    host: new LocalHost(root),
    sessionIdFactory: () => 'tool-abort-session',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition: AgentDefinition<Record<string, never>> = {
    name: 'shell-abort-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => createShellSessionTools(environment),
  }
  const provider = new StubProvider([
    [
      events.toolCall('call-1', 'shell_exec', {
        script: 'sh -c "sleep 0.2; printf leaked > tool-abort-leaked.txt"',
        yieldAfterMs: 1_000,
      }),
    ],
  ])
  const session = new AgentSession({
    provider,
    model,
    cwd: root,
    definition,
  })

  const send = session.send([{ type: 'text', text: 'run long command' }])
  await waitFor(() => environment.getSession('tool-abort-session') !== null)
  await session.abort()
  await send

  const settled = await environment.wait({ sessionId: 'tool-abort-session', yieldAfterMs: 1_000 })
  expect(settled.status).toBe('exited')
  await delay(250)
  await expect(access(leakedPath)).rejects.toThrow()
})

test('shell_wait observes AgentSession abort signals and terminates the foreground process', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-shell-tool-wait-abort-'))
  const leakedPath = join(root, 'tool-wait-abort-leaked.txt')
  const environment = new BashEnvironment({
    host: new LocalHost(root),
    sessionIdFactory: () => 'tool-wait-abort-session',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition: AgentDefinition<Record<string, never>> = {
    name: 'shell-wait-abort-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => createShellSessionTools(environment),
  }
  const provider = new StubProvider([
    [
      events.toolCall('call-1', 'shell_exec', {
        script: 'sh -c "sleep 0.2; printf leaked > tool-wait-abort-leaked.txt"',
        yieldAfterMs: 1,
      }),
    ],
    [
      events.toolCall('call-2', 'shell_wait', {
        sessionId: 'tool-wait-abort-session',
        yieldAfterMs: 1_000,
      }),
    ],
  ])
  const session = new AgentSession({ provider, model, cwd: root, definition })

  const send = session.send([{ type: 'text', text: 'wait for long command' }])
  await waitFor(() => {
    return session.transcript().pendingToolCalls().some((block) => block.toolName === 'shell_wait')
  })
  await session.abort()
  await send

  await delay(250)
  await expect(access(leakedPath)).rejects.toThrow()
})

test('shell_input observes AgentSession abort signals and terminates the foreground process', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-shell-tool-input-abort-'))
  const leakedPath = join(root, 'tool-input-abort-leaked.txt')
  const environment = new BashEnvironment({
    host: new LocalHost(root),
    sessionIdFactory: () => 'tool-input-abort-session',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition: AgentDefinition<Record<string, never>> = {
    name: 'shell-input-abort-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => createShellSessionTools(environment),
  }
  const provider = new StubProvider([
    [
      events.toolCall('call-1', 'shell_exec', {
        script: 'sh -c \'IFS= read -r line; sleep 0.2; printf leaked > tool-input-abort-leaked.txt\'',
        yieldAfterMs: 1,
      }),
    ],
    [
      events.toolCall('call-2', 'shell_input', {
        sessionId: 'tool-input-abort-session',
        stdin: 'go\n',
        yieldAfterMs: 1_000,
      }),
    ],
  ])
  const session = new AgentSession({ provider, model, cwd: root, definition })

  const send = session.send([{ type: 'text', text: 'send input to long command' }])
  await waitFor(() => {
    return session.transcript().pendingToolCalls().some((block) => block.toolName === 'shell_input')
  })
  await session.abort()
  await send

  await delay(250)
  await expect(access(leakedPath)).rejects.toThrow()
})

function latestToolResult(items: Array<{ type: string; output?: unknown }>): ShellToolResult {
  const item = [...items].reverse().find((candidate) => candidate.type === 'tool_result')
  if (!item || !Array.isArray(item.output)) throw new Error('missing tool_result')
  const first = item.output[0]
  if (!first || typeof first !== 'object' || !('type' in first) || first.type !== 'text') {
    throw new Error('tool_result was not text')
  }
  return JSON.parse((first as { text: string }).text) as ShellToolResult
}

async function waitFor(predicate: () => boolean): Promise<void> {
  const startedAt = Date.now()
  while (!predicate()) {
    if (Date.now() - startedAt > 1_000) throw new Error('Timed out waiting for predicate')
    await delay(1)
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

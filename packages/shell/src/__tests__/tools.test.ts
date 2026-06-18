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

test('toToolResult formats shell results while preserving metadata', () => {
  const result: ShellToolResult = {
    status: 'exited',
    shellId: 'tool-bigint-shell',
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

  expect(first.text).toContain('status: exited')
  expect(first.text).toContain('shellId: tool-bigint-shell')
  expect(first.text).toContain('exitCode: 0')
  expect(toolResult.metadata).toBe(result)
})

test('createShellSessionTools integrates shell_exec and shell_wait with AgentSession', async () => {
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'tool-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition: AgentDefinition<Record<string, never>> = {
    name: 'shell-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => createShellSessionTools(environment),
  }
  const shellExec = definition.tools({ agentSessionId: 'tool-agent', state: {}, cwd: process.cwd() }).find((tool) => tool.name === 'shell_exec')
  expect(shellExec?.description).toContain('foreground with yieldAfterMs')
  expect(shellExec?.description).toContain('instead of backgrounding and pkill/killall')
  const provider = new StubProvider([
    [events.toolCall('call-1', 'shell_exec', { script: 'sh -c "sleep 0.02; printf done"', yieldAfterMs: 1 })],
    (request: InferenceRequest) => {
      const result = latestToolResult(request.items)
      expect(result.status).toBe('running')
      return [events.toolCall('call-2', 'shell_wait', { shellId: result.shellId, yieldAfterMs: 1_000 })]
    },
    (request: InferenceRequest) => {
      const result = latestToolResult(request.items)
      expect(result.status).toBe('exited')
      expect(result.stdout).toBe('done')
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

test('shell_input requires non-empty stdin instead of polling', async () => {
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'tool-input-requires-stdin-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const tools = createShellSessionTools(environment)
  const shellInput = tools.find((tool) => tool.name === 'shell_input')
  if (!shellInput) throw new Error('missing shell_input tool')

  const ctx = {
    agentSessionId: 'tool-input-requires-stdin-agent',
    state: {},
    cwd: process.cwd(),
    toolCallId: 'call-1',
    signal: new AbortController().signal,
    emitProgress: () => {},
  }

  await expect(shellInput.invoke(ctx, { shellId: 'tool-input-requires-stdin-shell' })).rejects.toThrow(
    'shell_input requires string field "stdin"',
  )
  await expect(shellInput.invoke(ctx, { shellId: 'tool-input-requires-stdin-shell', stdin: '' })).rejects.toThrow(
    'shell_input field "stdin" must not be empty; use shell_wait to poll',
  )
})

test('shell_abort is an intentional control action, not a tool failure', async () => {
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'tool-intentional-abort-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const tools = createShellSessionTools(environment)
  const shellExec = tools.find((tool) => tool.name === 'shell_exec')
  const shellAbort = tools.find((tool) => tool.name === 'shell_abort')
  if (!shellExec || !shellAbort) throw new Error('missing shell tools')

  const ctx = {
    agentSessionId: 'tool-intentional-abort-agent',
    state: {},
    cwd: process.cwd(),
    toolCallId: 'call-1',
    signal: new AbortController().signal,
    emitProgress: () => {},
  }

  await shellExec.invoke(ctx, { script: 'sleep 10', yieldAfterMs: 1 })
  const aborted = await shellAbort.invoke({ ...ctx, toolCallId: 'call-2' }, { shellId: 'tool-intentional-abort-shell' })

  expect(textOutput(aborted)).toContain('status: aborted')
  expect(aborted.isError).toBe(false)
})

test('shell_exec observes AgentSession abort signals and terminates the foreground process', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-shell-tool-abort-'))
  const leakedPath = join(root, 'tool-abort-leaked.txt')
  const environment = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'tool-abort-shell',
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
  await waitFor(() => environment.getShell('tool-abort-shell') !== null)
  await session.abort()
  await send

  const settled = await environment.wait({ shellId: 'tool-abort-shell', yieldAfterMs: 1_000 })
  expect(settled.status).toBe('exited')
  await delay(250)
  await expect(access(leakedPath)).rejects.toThrow()
})

test('shell_wait observes AgentSession abort signals and terminates the foreground process', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-shell-tool-wait-abort-'))
  const leakedPath = join(root, 'tool-wait-abort-leaked.txt')
  const environment = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'tool-wait-abort-shell',
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
        shellId: 'tool-wait-abort-shell',
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
    shellIdFactory: () => 'tool-input-abort-shell',
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
        shellId: 'tool-input-abort-shell',
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

function latestToolResult(items: Array<{ type: string; output?: unknown }>): {
  status: string
  shellId: string
  stdout: string
} {
  const item = [...items].reverse().find((candidate) => candidate.type === 'tool_result')
  if (!item || !Array.isArray(item.output)) throw new Error('missing tool_result')
  const first = item.output[0]
  if (!first || typeof first !== 'object' || !('type' in first) || first.type !== 'text') {
    throw new Error('tool_result was not text')
  }
  const text = (first as { text: string }).text
  return {
    status: requiredField(text, 'status'),
    shellId: requiredField(text, 'shellId'),
    stdout: section(text, 'stdout'),
  }
}

function textOutput(result: { output: Array<{ type: string; text?: string }> }): string {
  const first = result.output[0]
  if (!first || first.type !== 'text' || typeof first.text !== 'string') throw new Error('expected text output')
  return first.text
}

function requiredField(text: string, name: string): string {
  const match = new RegExp(`^${name}: (.*)$`, 'm').exec(text)
  if (!match) throw new Error(`missing field ${name} in ${text}`)
  return match[1]
}

function section(text: string, name: string): string {
  const match = new RegExp(`^${name}:\\n([\\s\\S]*?)(?:\\n[a-zA-Z]+:|\\nnext:|$)`, 'm').exec(text)
  if (!match) throw new Error(`missing section ${name} in ${text}`)
  return match[1] === '(empty)' ? '' : match[1]
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

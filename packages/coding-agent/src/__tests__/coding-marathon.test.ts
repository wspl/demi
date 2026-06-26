import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { waitFor } from '@demi/utils'
import type { ModelSelection } from '@demi/core'
import {
  AgentSession,
  createStandardAgentTools,
  type AgentHarness,
  type AgentHarnessRuntime,
  type AgentToolInvokeContext,
  type AgentToolInvokeResult,
} from '@demi/agent'
import {
  BashEnvironment,
  CommandRegistry,
  type BashEnvironmentOptions,
} from '@demi/shell'
import { LocalHost } from '@demi/host-local'
import type { InferenceRequest } from '@demi/provider'
import { StubProvider, events } from '@demi/provider/testing'
import { createCodingAgentHarness } from '../index'

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

test('coding agent completes an editor/todo workflow through shell session tools', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-marathon-'))
  const host = new LocalHost(root)
  const harness = createCodingAgentHarness({ host })
  const { environment, runtime } = createRuntimeFromHarness(harness, root, {
    shellIdFactory: () => 'coding-shell',
  })
  const provider = new StubProvider([
    [
      events.toolCall('create-file', 'shell_exec', {
        timeoutMs: 1_000,
        script: "editor create src/app.ts <<'EOF'\nexport const value = 1\nEOF",
      }),
    ],
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.stdout).toBe('Created src/app.ts')
      return [events.toolCall('add-todo', 'shell_exec', { shellId: result.shellId, script: 'todo add \"Run tests\" --json', timeoutMs: 1_000 })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(JSON.parse(result.stdout)).toEqual({ todo: { id: 'T1', text: 'Run tests', status: 'pending' } })
      return [
        events.toolCall('edit-file', 'shell_exec', {
          shellId: result.shellId,
          timeoutMs: 1_000,
          script: 'editor edit src/app.ts --old "1" --new "2"',
        }),
      ]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.stdout).toBe('Edited src/app.ts')
      return [events.text('done'), events.response()]
    },
  ])
  const session = new AgentSession({ provider, model, cwd: root, runtime })

  await session.send([{ type: 'text', text: 'Create app file, track tests, then update value.' }])

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'tool_call',
    'tool_call',
    'text',
    'response',
  ])
  const file = await environment.exec({ shellId: 'coding-shell', script: 'cat src/app.ts' })
  expect(file.stdout.delta).toBe('export const value = 2\n')
  const todos = await environment.exec({ agentSessionId: session.id(), shellId: 'coding-shell', script: 'todo list --json' })
  expect(JSON.parse(todos.stdout.delta)).toEqual({ todos: [{ id: 'T1', text: 'Run tests', status: 'pending' }] })
})

test('coding agent preserves workflow state across multiple user messages', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-multiturn-'))
  const harness = createCodingAgentHarness({ host: new LocalHost(root) })
  const { environment, runtime } = createRuntimeFromHarness(harness, root, {
    shellIdFactory: () => 'coding-multiturn-shell',
  })
  const provider = new StubProvider([
    [
      events.toolCall('start-workflow', 'shell_exec', {
        timeoutMs: 1_000,
        script: [
          "editor create note.txt <<'EOF'",
          'first turn',
          'EOF',
          'todo add "carry state" --json',
        ].join('\n'),
      }),
    ],
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.stdout).toContain('Created note.txt')
      expect(result.stdout).toContain('"text":"carry state"')
      return [events.text('first turn complete'), events.response()]
    },
    (request: InferenceRequest) => {
      const serialized = JSON.stringify(request.items)
      expect(serialized).toContain('Start the workflow.')
      expect(serialized).toContain('Continue the workflow and close the todo.')
      expect(serialized).toContain('first turn complete')
      expect(serialized).toContain('carry state')
      return [
        events.toolCall('continue-workflow', 'shell_exec', {
          timeoutMs: 1_000,
          script: ['todo done T1 --json', "printf '\\n'", 'todo list --json', "printf '\\n'", 'cat note.txt'].join('\n'),
        }),
      ]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.stdout).toContain('"status":"done"')
      expect(result.stdout).toContain('"todos":[{"id":"T1","text":"carry state","status":"done"}]')
      expect(result.stdout).toContain('first turn')
      return [events.text('second turn complete'), events.response()]
    },
  ])
  const session = new AgentSession({ provider, model, cwd: root, runtime }, { agentSessionId: 'coding-multiturn-agent' })

  await session.send([{ type: 'text', text: 'Start the workflow.' }])
  await session.send([{ type: 'text', text: 'Continue the workflow and close the todo.' }])

  expect(provider.consumedTurns).toBe(4)
  const todos = await environment.exec({ agentSessionId: 'coding-multiturn-agent', script: 'todo list --json' })
  expect(JSON.parse(todos.stdout.delta)).toEqual({
    todos: [{ id: 'T1', text: 'carry state', status: 'done' }],
  })
})

test('coding agent preserves cwd and env when reusing a shell session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-shell-state-'))
  const harness = createCodingAgentHarness({ host: new LocalHost(root) })
  const { runtime } = createRuntimeFromHarness(harness, root, {
    shellIdFactory: () => 'coding-state-shell',
  })
  const provider = new StubProvider([
    [
      events.toolCall('prepare-shell-state', 'shell_exec', {
        timeoutMs: 1_000,
        script: [
          'mkdir -p pkg',
          'cd pkg',
          'export WORKFLOW_TOKEN=kept',
          'printf "prepared:%s:%s" "$PWD" "$WORKFLOW_TOKEN"',
        ].join('\n'),
      }),
    ],
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.stdout).toBe(`prepared:${join(root, 'pkg')}:kept`)
      return [
        events.toolCall('read-shell-state', 'shell_exec', {
          shellId: result.shellId,
          timeoutMs: 1_000,
          script: 'printf "state:%s:%s" "$PWD" "$WORKFLOW_TOKEN"',
        }),
      ]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.stdout).toBe(`state:${join(root, 'pkg')}:kept`)
      return [events.text('state preserved'), events.response()]
    },
  ])
  const session = new AgentSession({ provider, model, cwd: root, runtime })

  await session.send([{ type: 'text', text: 'Prepare shell state, then reuse it.' }])

  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
})

test('coding agent iterates from a failing project test to a passing fix', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-test-fix-'))
  const host = new LocalHost(root)
  const harness = createCodingAgentHarness({ host })
  const { environment, runtime } = createRuntimeFromHarness(harness, root, {
    shellIdFactory: () => 'coding-fix-shell',
  })
  const provider = new StubProvider([
    [
      events.toolCall('create-project', 'shell_exec', {
        timeoutMs: 1_000,
        script: [
          'mkdir -p src',
          "editor create src/todo.ts <<'EOF'\nexport function addTodo(items: string[], text: string): string[] {\n  return items\n}\nEOF",
          "editor create src/todo.test.ts <<'EOF'\nimport { expect, test } from 'bun:test'\nimport { addTodo } from './todo'\n\ntest('adds a todo item', () => {\n  expect(addTodo([], 'ship tests')).toEqual(['ship tests'])\n})\nEOF",
        ].join('\n'),
      }),
    ],
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.exitCode).toBe(0)
      expect(result.stdout).toContain('Created src/todo.ts')
      return [events.toolCall('run-failing-tests', 'shell_exec', { shellId: result.shellId, script: 'bun test src/todo.test.ts', timeoutMs: 1_000 })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.exitCode).not.toBe(0)
      expect(`${result.stdout}\n${result.stderr}`).toContain('ship tests')
      return [events.toolCall('read-source', 'shell_exec', { shellId: result.shellId, script: 'cat src/todo.ts', timeoutMs: 1_000 })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.stdout).toContain('return items')
      return [
        events.toolCall('fix-source', 'shell_exec', {
          shellId: result.shellId,
          timeoutMs: 1_000,
          script: 'editor edit src/todo.ts --old "return items" --new "return [...items, text]"',
        }),
      ]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.exitCode).toBe(0)
      return [events.toolCall('run-passing-tests', 'shell_exec', { shellId: result.shellId, script: 'bun test src/todo.test.ts', timeoutMs: 1_000 })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.exitCode).toBe(0)
      expect(`${result.stdout}\n${result.stderr}`).toContain('1 pass')
      return [events.text('fixed'), events.response()]
    },
  ])
  const session = new AgentSession({ provider, model, cwd: root, runtime })

  await session.send([{ type: 'text', text: 'Create a tiny todo module, run its test, fix the failure, and rerun.' }])

  const file = await environment.exec({ shellId: 'coding-fix-shell', script: 'cat src/todo.ts' })
  expect(file.stdout.delta).toContain('return [...items, text]')
})

test('coding agent controls a long foreground command with status and abort', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-long-command-'))
  const harness = createCodingAgentHarness({ host: new LocalHost(root) })
  const { runtime } = createRuntimeFromHarness(harness, root, {
    shellIdFactory: () => 'coding-long-shell',
  })
  const provider = new StubProvider([
    [
      events.toolCall('start-long', 'shell_exec', {
        script: "sh -c 'printf ready; sleep 10'",
        timeoutMs: 20,
      }),
    ],
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('running')
      expect(result.stdout).toContain('ready')
      return [events.toolCall('stop-long', 'shell_abort', { commandId: result.commandId })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('aborted')
      return [events.text('stopped'), events.response()]
    },
  ])
  const session = new AgentSession({ provider, model, cwd: root, runtime })

  await session.send([{ type: 'text', text: 'Run the interactive long command, feed it input, then stop it.' }])

  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
})

test('coding agent exercises all standard shell control tools in one flow', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-standard-tools-'))
  const harness = createCodingAgentHarness({ host: new LocalHost(root) })
  let session: AgentSession<Record<string, never>> | null = null
  const { runtime } = createRuntimeFromHarness(
    harness,
    root,
    {
      shellIdFactory: () => `coding-standard-shell-${crypto.randomUUID()}`,
    },
    (_ctx, durationMs) => {
      if (!session) throw new Error('session is not ready')
      return session.scheduleYieldWakeup(durationMs)
    },
  )
  let readerCommandId = ''
  let longCommandId = ''
  const provider = new StubProvider([
    [
      events.toolCall('start-reader', 'shell_exec', {
        description: 'Start reader',
        script: 'sh -c \'IFS= read -r line; printf "DEMI_FULL_INPUT:%s" "$line"\'',
        timeoutMs: 1,
      }),
    ],
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('running')
      readerCommandId = result.commandId ?? ''
      return [events.toolCall('yield-reader', 'yield', { description: 'Wait before checking reader', durationMs: 1 })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.commandId).toBe(readerCommandId)
      expect(result.status).toBe('running')
      return [events.toolCall('status-reader', 'shell_status', { description: 'Check reader', commandId: readerCommandId })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.commandId).toBe(readerCommandId)
      expect(result.status).toBe('running')
      return [events.toolCall('write-reader', 'shell_write', { description: 'Send reader input', commandId: readerCommandId, stdin: 'typed\n' })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.commandId).toBe(readerCommandId)
      return [events.toolCall('yield-reader-after-write', 'yield', { description: 'Wait for reader output', durationMs: 5 })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.commandId).toBe(readerCommandId)
      return [events.toolCall('status-reader-after-write', 'shell_status', { commandId: readerCommandId })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      // The reader has exited with complete output, so its handle is released — no commandId now.
      expect(result.status).toBe('exited')
      expect(result.stdout).toContain('DEMI_FULL_INPUT:typed')
      return [
        events.toolCall('start-long', 'shell_exec', {
          description: 'Start long command',
          script: "sh -c 'printf long-ready; sleep 10'",
          timeoutMs: 20,
        }),
      ]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('running')
      expect(result.stdout).toContain('long-ready')
      longCommandId = result.commandId ?? ''
      return [events.toolCall('status-long', 'shell_status', { description: 'Check long command', commandId: longCommandId })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.commandId).toBe(longCommandId)
      expect(result.status).toBe('running')
      return [events.toolCall('abort-long', 'shell_abort', { description: 'Stop long command', commandId: longCommandId })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      // The aborted command's output is complete, so its handle is released — no commandId now.
      expect(result.status).toBe('aborted')
      return [events.text('full control ok'), events.response()]
    },
  ])
  session = new AgentSession({ provider, model, cwd: root, runtime })

  await session.send([{ type: 'text', text: 'Exercise every standard shell control tool.' }])
  await waitFor(
    () => session?.transcript().blocks.some((block) => block.type === 'text' && block.text.includes('full control ok')) ?? false,
    () => transcriptSummary(session),
  )

  const toolNames = session.transcript().blocks.flatMap((block) => (block.type === 'tool_call' ? [block.toolName] : []))
  expect(new Set(toolNames)).toEqual(new Set(['shell_exec', 'yield', 'shell_status', 'shell_write', 'shell_abort']))
  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
})

function createRuntimeFromHarness(
  harness: AgentHarness<Record<string, never>>,
  cwd: string,
  options: Omit<BashEnvironmentOptions, 'host' | 'commands'> = {},
  scheduleYield?: (
    ctx: AgentToolInvokeContext<Record<string, never>>,
    durationMs: number,
  ) => AgentToolInvokeResult,
): { environment: BashEnvironment; runtime: AgentHarnessRuntime<Record<string, never>> } {
  const state = harness.initialState()
  const harnessContext = { state, cwd }
  const registry = new CommandRegistry()
  for (const command of harness.commands?.(harnessContext) ?? []) registry.register(command)
  const environment = new BashEnvironment({
    initialEnv: { PATH: process.env.PATH ?? '' },
    ...options,
    host: harness.host(harnessContext),
    commands: registry,
  })
  const runtime: AgentHarnessRuntime<Record<string, never>> = {
    harnessName: harness.name,
    initialState: () => state,
    systemPrompt: (ctx) => harness.systemPrompt(ctx),
    preamble: (ctx) => harness.preamble?.(ctx) ?? null,
    resolveReferences: (ctx, content) => harness.resolveReferences?.(ctx, content) ?? content,
    lifecycle: (event) => harness.lifecycle?.(event),
    tools: () =>
      createStandardAgentTools({
        environment,
        scheduleYield:
          scheduleYield ??
          ((_ctx, durationMs) => ({
            output: [{ type: 'text', text: `yield scheduled\nwakeupId: test\ndurationMs: ${durationMs}` }],
            stopAfterToolResult: true,
          })),
      }),
  }
  return { environment, runtime }
}

function latestShellResult(request: InferenceRequest): {
  status: string
  shellId: string | undefined
  commandId: string | undefined
  stdout: string
  stderr: string
  exitCode: number | null
} {
  const item = [...request.items].reverse().find((candidate) => {
    if (candidate.type !== 'tool_result') return false
    const [first] = candidate.output
    return first?.type === 'text' && first.text.includes('status:')
  })
  if (item?.type !== 'tool_result') throw new Error('missing tool result')
  const [first] = item.output
  if (first?.type !== 'text') throw new Error('tool result was not text')
  const exitCodeText = optionalField(first.text, 'exitCode')
  return {
    status: requiredField(first.text, 'status'),
    // Completed short commands release the handle, so shellId/commandId are present only while
    // the command is still running or its output was truncated.
    shellId: optionalField(first.text, 'shellId') ?? undefined,
    commandId: optionalField(first.text, 'commandId') ?? undefined,
    // The tool result no longer carries separate stdout/stderr sections — the model-visible output
    // is one budgeted preview. `stdout` exposes that preview; `stderr` stays empty.
    stdout: previewSection(first.text),
    stderr: '',
    exitCode: exitCodeText === null ? null : Number(exitCodeText),
  }
}

function requiredField(text: string, name: string): string {
  const match = new RegExp(`^${name}: (.*)$`, 'm').exec(text)
  if (!match) throw new Error(`missing field ${name} in ${text}`)
  return match[1]
}

function optionalField(text: string, name: string): string | null {
  const match = new RegExp(`^${name}: (.*)$`, 'm').exec(text)
  return match?.[1] ?? null
}

function previewSection(text: string): string {
  const marker = 'preview:\n'
  const start = text.indexOf(marker)
  if (start === -1) return ''
  const rest = text.slice(start + marker.length)
  const boundary = /\n(?:previewTruncated|next):/.exec(rest)
  const rawValue = boundary ? rest.slice(0, boundary.index) : rest
  return rawValue.endsWith('\n') ? rawValue.slice(0, -1) : rawValue
}

function transcriptSummary(session: AgentSession<Record<string, never>> | null): string {
  if (!session) return 'session is null'
  return session
    .transcript()
    .blocks.map((block) => {
      if (block.type === 'tool_call') return `${block.type}:${block.toolName}:${block.status}`
      if (block.type === 'text') return `${block.type}:${block.text}`
      if (block.type === 'error') return `${block.type}:${block.message}`
      return block.type
    })
    .join(' | ')
}

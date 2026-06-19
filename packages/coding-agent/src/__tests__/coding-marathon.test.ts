import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { AgentSession, type AgentHarness, type AgentHarnessRuntime } from '@demi/agent'
import {
  BashEnvironment,
  CommandRegistry,
  createShellSessionTools,
  type BashEnvironmentOptions,
} from '@demi/shell'
import { LocalHost } from '@demi/shell/local-host'
import { StubProvider, events, type InferenceRequest } from '@demi/provider'
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
        script: "editor create src/app.ts <<'EOF'\nexport const value = 1\nEOF",
      }),
    ],
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.stdout).toBe('Created src/app.ts')
      return [events.toolCall('add-todo', 'shell_exec', { shellId: result.shellId, script: 'todo add "Run tests" --json' })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(JSON.parse(result.stdout)).toEqual({ todo: { id: 'T1', text: 'Run tests', status: 'pending' } })
      return [
        events.toolCall('edit-file', 'shell_exec', {
          shellId: result.shellId,
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
  expect(file.output.stdoutDelta).toBe('export const value = 2\n')
  const todos = await environment.exec({ agentSessionId: session.id(), shellId: 'coding-shell', script: 'todo list --json' })
  expect(JSON.parse(todos.output.stdoutDelta)).toEqual({ todos: [{ id: 'T1', text: 'Run tests', status: 'pending' }] })
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
          script: ['todo done T1 --json', "printf '\\n'", 'todo list --json', "printf '\\n'", 'cat note.txt'].join('\n'),
        }),
      ]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.shellId).toBe('coding-multiturn-shell')
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
  expect(JSON.parse(todos.output.stdoutDelta)).toEqual({
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
          script: 'printf "state:%s:%s" "$PWD" "$WORKFLOW_TOKEN"',
        }),
      ]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.shellId).toBe('coding-state-shell')
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
      return [events.toolCall('run-failing-tests', 'shell_exec', { shellId: result.shellId, script: 'bun test src/todo.test.ts' })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.exitCode).not.toBe(0)
      expect(`${result.stdout}\n${result.stderr}`).toContain('ship tests')
      return [events.toolCall('read-source', 'shell_exec', { shellId: result.shellId, script: 'cat src/todo.ts' })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.stdout).toContain('return items')
      return [
        events.toolCall('fix-source', 'shell_exec', {
          shellId: result.shellId,
          script: 'editor edit src/todo.ts --old "return items" --new "return [...items, text]"',
        }),
      ]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('exited')
      expect(result.exitCode).toBe(0)
      return [events.toolCall('run-passing-tests', 'shell_exec', { shellId: result.shellId, script: 'bun test src/todo.test.ts' })]
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
  expect(file.output.stdoutDelta).toContain('return [...items, text]')
})

test('coding agent controls a long foreground command with wait, input, and abort', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-long-command-'))
  const harness = createCodingAgentHarness({ host: new LocalHost(root) })
  const { runtime } = createRuntimeFromHarness(harness, root, {
    shellIdFactory: () => 'coding-long-shell',
  })
  const provider = new StubProvider([
    [
      events.toolCall('start-long', 'shell_exec', {
        script: "sh -c 'sleep 0.02; printf ready; IFS= read -r line; printf \" got:%s\" \"$line\"; sleep 10'",
        yieldAfterMs: 1,
      }),
    ],
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('running')
      return [events.toolCall('wait-ready', 'shell_wait', { shellId: result.shellId, yieldAfterMs: 1_000 })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('running')
      expect(result.stdout).toContain('ready')
      return [events.toolCall('send-input', 'shell_input', { shellId: result.shellId, stdin: 'typed\n', yieldAfterMs: 20 })]
    },
    (request: InferenceRequest) => {
      const result = latestShellResult(request)
      expect(result.status).toBe('running')
      expect(result.stdout).toContain('got:typed')
      return [events.toolCall('stop-long', 'shell_abort', { shellId: result.shellId })]
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

function createRuntimeFromHarness(
  harness: AgentHarness<Record<string, never>>,
  cwd: string,
  options: Omit<BashEnvironmentOptions, 'host' | 'commands'> = {},
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
    tools: () => createShellSessionTools(environment),
  }
  return { environment, runtime }
}

function latestShellResult(request: InferenceRequest): {
  status: string
  shellId: string
  stdout: string
  stderr: string
  exitCode: number | null
} {
  const item = [...request.items].reverse().find((candidate) => candidate.type === 'tool_result')
  if (item?.type !== 'tool_result') throw new Error('missing tool result')
  const [first] = item.output
  if (first?.type !== 'text') throw new Error('tool result was not text')
  const exitCodeText = optionalField(first.text, 'exitCode')
  return {
    status: requiredField(first.text, 'status'),
    shellId: requiredField(first.text, 'shellId'),
    stdout: section(first.text, 'stdout'),
    stderr: section(first.text, 'stderr'),
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

function section(text: string, name: string): string {
  const start = text.indexOf(`${name}:\n`)
  if (start === -1) throw new Error(`missing section ${name} in ${text}`)
  const bodyStart = start + `${name}:\n`.length
  const rest = text.slice(bodyStart)
  const nextField = /\n(?:stdout|stderr|status|shellId|exitCode|runningMs|reason|idleMs|next):/.exec(rest)
  const rawValue = nextField ? rest.slice(0, nextField.index) : rest
  const value = nextField && rawValue.endsWith('\n') ? rawValue.slice(0, -1) : rawValue
  return value === '(empty)' ? '' : value
}

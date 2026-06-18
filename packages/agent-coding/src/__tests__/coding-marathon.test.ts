import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { AgentSession } from '@demi/base-agent'
import { BashEnvironment } from '@demi/shell'
import { LocalHost } from '@demi/shell/local-host'
import { StubProvider, events, type InferenceRequest } from '@demi/provider'
import { createCodingAgentDefinition } from '../index'

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
  const environment = new BashEnvironment({
    host,
    shellIdFactory: () => 'coding-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition = createCodingAgentDefinition({ environment })
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
  const session = new AgentSession({ provider, model, cwd: root, definition })

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

function latestShellResult(request: InferenceRequest): { status: string; shellId: string; stdout: string } {
  const item = [...request.items].reverse().find((candidate) => candidate.type === 'tool_result')
  if (item?.type !== 'tool_result') throw new Error('missing tool result')
  const [first] = item.output
  if (first?.type !== 'text') throw new Error('tool result was not text')
  return {
    status: requiredField(first.text, 'status'),
    shellId: requiredField(first.text, 'shellId'),
    stdout: section(first.text, 'stdout'),
  }
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

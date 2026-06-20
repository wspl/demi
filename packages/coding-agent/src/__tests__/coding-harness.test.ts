import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { AgentSession, type AgentHarness, type AgentHarnessRuntime } from '@demi/agent'
import type { InferenceRequest } from '@demi/provider'
import { StubProvider, events } from '@demi/provider/testing'
import {
  BashEnvironment,
  CommandRegistry,
  createShellSessionTools,
  type Host,
  type HostDirent,
  type HostFileSystem,
} from '@demi/shell'
import { LocalHost } from '@demi/host-local'
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

test('coding agent harness exposes shell session tools and registered command prompt', async () => {
  const harness = createCodingAgentHarness({ host: new LocalHost(process.cwd()) })
  const state = harness.initialState()
  const commands = harness.commands?.({ state, cwd: process.cwd() }) ?? []
  const { environment, runtime } = createRuntimeFromHarness(harness, process.cwd())

  expect(harness.name).toBe('coding')
  expect(commands.map((command) => command.name)).toEqual(['editor', 'todo'])
  expect(runtime.tools({ agentSessionId: 'coding-test-agent', state, cwd: process.cwd() }).map((tool) => tool.name)).toEqual([
    'shell_exec',
    'shell_wait',
    'shell_input',
    'shell_abort',
  ])
  const prompt = harness.systemPrompt({
    agentSessionId: 'coding-test-agent',
    state,
    cwd: process.cwd(),
    transcript: {} as never,
  })
  expect(prompt).toContain('editor: Create, edit, and patch workspace files.')
  expect(prompt).toContain('Treat cwd as the task workspace')
  expect(prompt).toContain('do not create a separate project directory under /tmp')
  expect(prompt).toContain('editor create')
  expect(prompt).toContain('Effects: modifies workspace files by creating a new file')
  expect(prompt).toContain('Success output: writes "Created <path>" to stdout')
  expect(prompt).toContain('Failure output: writes the reason to stderr and exits non-zero')
  expect(prompt).toContain('todo: Manage an agent-session-scoped task list')
  expect(prompt).toContain('todo add "Run tests"')
  expect(prompt).toContain('Effects: modifies agent-session-scoped command storage')
  expect(prompt).toContain('run them in the foreground with a short yieldAfterMs')
  expect(prompt).toContain('do not set timeoutMs on shell_wait for a process you intend to keep running')
  expect(prompt).toContain('avoid pkill/killall by process name')
  expect(prompt).toContain('instead of restarting it to demonstrate the same behavior again')
  expect(prompt).toContain('include a newline such as "Alice\\n" for line-oriented prompts')
  expect(prompt).toContain('do not rely on the session script builtin read across turns')
  expect(prompt).toContain('File references attached by the client are expanded before provider calls.')

  const todo = await environment.exec({ script: 'todo add "Verify default registration"' })
  expect(todo.output.stdoutDelta).toBe('[ ] T1 Verify default registration\n')
  const editorPrompt = await environment.exec({ shellId: todo.shellId, script: 'editor prompt' })
  expect(editorPrompt.output.stdoutDelta).toContain('editor create')
  expect(editorPrompt.output.stdoutDelta).toContain('Effects: modifies workspace files by creating a new file')
  expect(editorPrompt.output.stdoutDelta).toContain('Success output: writes "Created <path>" to stdout')
})

test('coding agent resolves file references through the workspace host', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-refs-'))
  await writeFile(join(root, 'note.txt'), 'hello from file\n', 'utf8')
  const spacedPath = join(root, 'space note.txt')
  await writeFile(spacedPath, 'hello from encoded file URL\n', 'utf8')
  const harness = createCodingAgentHarness({ host: new LocalHost(root) })
  if (!harness.resolveReferences) throw new Error('expected resolveReferences')

  const resolved = await harness.resolveReferences(
    {
      agentSessionId: 'coding-ref-agent',
      state: harness.initialState(),
      cwd: root,
      transcript: {} as never,
      signal: new AbortController().signal,
    },
    [{ type: 'text', text: 'read this' }, { type: 'reference', reference: 'note.txt' }],
  )

  expect(resolved).toEqual([
    { type: 'text', text: 'read this' },
    { type: 'text', text: '<file path="note.txt">\nhello from file\n\n</file>' },
  ])

  const resolvedUrl = await harness.resolveReferences(
    {
      agentSessionId: 'coding-ref-agent',
      state: harness.initialState(),
      cwd: root,
      transcript: {} as never,
      signal: new AbortController().signal,
    },
    [{ type: 'reference', reference: `file://${spacedPath.replaceAll(' ', '%20')}` }],
  )

  expect(resolvedUrl).toEqual([
    { type: 'text', text: `<file path="${spacedPath}">\nhello from encoded file URL\n\n</file>` },
  ])
})

test('coding agent file references read through Host.fs', async () => {
  const host = new RecordingHost('/workspace', 'hello from fake host\n')
  const harness = createCodingAgentHarness({ host })
  if (!harness.resolveReferences) throw new Error('expected resolveReferences')

  const resolved = await harness.resolveReferences(
    {
      agentSessionId: 'coding-ref-agent',
      state: harness.initialState(),
      cwd: '/workspace',
      transcript: {} as never,
      signal: new AbortController().signal,
    },
    [{ type: 'reference', reference: 'note.txt' }],
  )

  expect(resolved).toEqual([{ type: 'text', text: '<file path="note.txt">\nhello from fake host\n\n</file>' }])
  expect(host.fs.calls).toEqual([['readFile', 'note.txt', '/workspace']])
  expect(host.spawnCalls).toBe(0)
})

test('coding agent resolves file references before AgentSession sends the provider request', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-session-refs-'))
  await writeFile(join(root, 'note.txt'), 'hello from session file\n', 'utf8')
  const harness = createCodingAgentHarness({ host: new LocalHost(root) })
  const { runtime } = createRuntimeFromHarness(harness, root)
  const provider = new StubProvider([
    (request: InferenceRequest) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [
            { type: 'text', text: 'inspect this file' },
            { type: 'text', text: '<file path="note.txt">\nhello from session file\n\n</file>' },
          ],
        },
      ])
      expect(JSON.stringify(request.items)).not.toContain('"reference"')
      return [events.text('read file'), events.response()]
    },
  ])
  const session = new AgentSession({ provider, model, cwd: root, runtime }, { agentSessionId: 'coding-ref-session' })

  await session.send([{ type: 'text', text: 'inspect this file' }, { type: 'reference', reference: 'note.txt' }])

  expect(session.transcript().blocks[0]).toMatchObject({
    type: 'user',
    content: [
      { type: 'text', text: 'inspect this file' },
      { type: 'text', text: '<file path="note.txt">\nhello from session file\n\n</file>' },
    ],
  })
})

test('coding agent rejects file references outside the workspace host root', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-ref-root-'))
  const outside = await mkdtemp(join(tmpdir(), 'demi-coding-ref-outside-'))
  const outsidePath = join(outside, 'secret.txt')
  await writeFile(outsidePath, 'outside\n', 'utf8')
  const harness = createCodingAgentHarness({ host: new LocalHost(root) })
  if (!harness.resolveReferences) throw new Error('expected resolveReferences')

  await expect(
    harness.resolveReferences(
      {
        agentSessionId: 'coding-ref-agent',
        state: harness.initialState(),
        cwd: root,
        transcript: {} as never,
        signal: new AbortController().signal,
      },
      [{ type: 'reference', reference: outsidePath }],
    ),
  ).rejects.toThrow('File reference escapes workspace')
})

test('coding agent harness leaves shell lifecycle to host assembly', () => {
  const harness = createCodingAgentHarness({ host: new LocalHost(process.cwd()) })

  expect('tools' in harness).toBe(false)
  expect(harness.dispose).toBeUndefined()
})

function createRuntimeFromHarness(
  harness: AgentHarness<Record<string, never>>,
  cwd: string,
): { environment: BashEnvironment; runtime: AgentHarnessRuntime<Record<string, never>>; state: Record<string, never> } {
  const state = harness.initialState()
  const harnessContext = { state, cwd }
  const registry = new CommandRegistry()
  for (const command of harness.commands?.(harnessContext) ?? []) registry.register(command)
  const environment = new BashEnvironment({
    host: harness.host(harnessContext),
    commands: registry,
    initialEnv: { PATH: process.env.PATH ?? '' },
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
  return { environment, runtime, state }
}

class RecordingHost implements Host {
  readonly fs: RecordingFileSystem
  spawnCalls = 0

  constructor(
    readonly root: string,
    stdoutText: string,
  ) {
    this.fs = new RecordingFileSystem(stdoutText)
  }

  async spawn(): Promise<never> {
    this.spawnCalls += 1
    throw new Error('Host.spawn must not be used for file references')
  }
}

class RecordingFileSystem implements HostFileSystem {
  readonly calls: unknown[][] = []

  constructor(private readonly text: string) {}

  async readFile(path: string, options?: { cwd?: string }): Promise<Uint8Array> {
    this.calls.push(['readFile', path, options?.cwd])
    return new TextEncoder().encode(this.text)
  }

  async writeFile(): Promise<void> { throw new Error('not implemented') }
  async appendFile(): Promise<void> { throw new Error('not implemented') }
  async exists(): Promise<boolean> { throw new Error('not implemented') }
  async stat(): Promise<never> { throw new Error('not implemented') }
  async lstat(): Promise<never> { throw new Error('not implemented') }
  async readdir(path: string, options: { cwd?: string; withFileTypes: true }): Promise<HostDirent[]>
  async readdir(path: string, options?: { cwd?: string; withFileTypes?: false }): Promise<string[]>
  async readdir(): Promise<string[] | HostDirent[]> { throw new Error('not implemented') }
  async mkdir(): Promise<void> { throw new Error('not implemented') }
  async rm(): Promise<void> { throw new Error('not implemented') }
  async cp(): Promise<void> { throw new Error('not implemented') }
  async mv(): Promise<void> { throw new Error('not implemented') }
  async chmod(): Promise<void> { throw new Error('not implemented') }
  async symlink(): Promise<void> { throw new Error('not implemented') }
  async link(): Promise<void> { throw new Error('not implemented') }
  async readlink(): Promise<string> { throw new Error('not implemented') }
  async realpath(): Promise<string> { throw new Error('not implemented') }
  async utimes(): Promise<void> { throw new Error('not implemented') }
}

import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { BashEnvironment } from '@demi/shell'
import { LocalHost } from '@demi/shell/local-host'
import { createCodingAgentDefinition } from '../index'

test('coding agent definition exposes shell session tools and registered command prompt', async () => {
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition = createCodingAgentDefinition({ environment })
  const state = definition.initialState()

  expect(definition.name).toBe('coding')
  expect(definition.commands?.({ state, cwd: process.cwd() }).map((command) => command.name)).toEqual(['editor', 'todo'])
  expect(definition.tools({ state, cwd: process.cwd() }).map((tool) => tool.name)).toEqual([
    'shell_exec',
    'shell_wait',
    'shell_input',
    'shell_abort',
  ])
  const prompt = definition.systemPrompt({
    state,
    cwd: process.cwd(),
    transcript: {} as never,
  })
  expect(prompt).toContain('editor: Create, edit, and patch workspace files.')
  expect(prompt).toContain('editor create')
  expect(prompt).toContain('Effects: modifies workspace files by creating a new file')
  expect(prompt).toContain('Success output: writes "Created <path>" to stdout')
  expect(prompt).toContain('Failure output: writes the reason to stderr and exits non-zero')
  expect(prompt).toContain('todo: Manage a session-scoped task list')
  expect(prompt).toContain('todo add "Run tests"')
  expect(prompt).toContain('Effects: modifies session-scoped command storage')
  expect(prompt).toContain('File references attached by the client are expanded before provider calls.')

  const todo = await environment.exec({ script: 'todo add "Verify default registration"' })
  expect(todo.output.stdoutDelta).toBe('[ ] T1 Verify default registration\n')
  const editorPrompt = await environment.exec({ sessionId: todo.sessionId, script: 'editor prompt' })
  expect(editorPrompt.output.stdoutDelta).toContain('editor create')
  expect(editorPrompt.output.stdoutDelta).toContain('Effects: modifies workspace files by creating a new file')
  expect(editorPrompt.output.stdoutDelta).toContain('Success output: writes "Created <path>" to stdout')
})

test('coding agent resolves file references through the workspace host', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-refs-'))
  await writeFile(join(root, 'note.txt'), 'hello from file\n', 'utf8')
  const spacedPath = join(root, 'space note.txt')
  await writeFile(spacedPath, 'hello from encoded file URL\n', 'utf8')
  const environment = new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition = createCodingAgentDefinition({ environment })
  if (!definition.resolveReferences) throw new Error('expected resolveReferences')

  const resolved = await definition.resolveReferences(
    {
      state: definition.initialState(),
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

  const resolvedUrl = await definition.resolveReferences(
    {
      state: definition.initialState(),
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

test('coding agent rejects file references outside the workspace host root', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-coding-ref-root-'))
  const outside = await mkdtemp(join(tmpdir(), 'demi-coding-ref-outside-'))
  const outsidePath = join(outside, 'secret.txt')
  await writeFile(outsidePath, 'outside\n', 'utf8')
  const environment = new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition = createCodingAgentDefinition({ environment })
  if (!definition.resolveReferences) throw new Error('expected resolveReferences')

  await expect(
    definition.resolveReferences(
      {
        state: definition.initialState(),
        cwd: root,
        transcript: {} as never,
        signal: new AbortController().signal,
      },
      [{ type: 'reference', reference: outsidePath }],
    ),
  ).rejects.toThrow('File reference escapes workspace')
})

test('coding agent dispose clears shell sessions owned by its environment', async () => {
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    sessionIdFactory: () => 'coding-dispose-session',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition = createCodingAgentDefinition({ environment })
  const state = definition.initialState()

  const created = await environment.exec({ script: 'printf live' })
  expect(environment.getSession(created.sessionId)).not.toBeNull()

  await definition.dispose?.({
    state,
    cwd: process.cwd(),
    transcript: {} as never,
  })

  expect(environment.getSession(created.sessionId)).toBeNull()
})

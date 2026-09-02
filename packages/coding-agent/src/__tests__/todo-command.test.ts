import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { type ShellEnvironment } from '@demicodes/shell'
import { hostlessShell } from '@demicodes/host-virtual/testing'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { createCodingCommandRegistry } from '../index'

test('todo command supports add/list/update/done with raw and JSON output', async () => {
  const env = await createTodoEnvironment(() => 'todo-shell')

  const add = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo add "Run tests"' })
  expect(add.stdout.delta).toBe('[ ] T1 Run tests\n')

  const addJson = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo add "Write docs" --json' })
  expect(JSON.parse(addJson.stdout.delta)).toEqual({
    todo: { id: 'T2', text: 'Write docs', status: 'pending' },
  })

  const rawList = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo list' })
  expect(rawList.stdout.delta).toBe('[ ] T1 Run tests\n[ ] T2 Write docs\n')

  const update = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo update T1 --text "Run full tests"' })
  expect(update.stdout.delta).toBe('[ ] T1 Run full tests\n')

  const inProgress = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo update T1 --status in_progress --json' })
  expect(JSON.parse(inProgress.stdout.delta)).toEqual({
    todo: { id: 'T1', text: 'Run full tests', status: 'in_progress' },
  })

  const doneRaw = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo done T2' })
  expect(doneRaw.stdout.delta).toBe('[x] T2 Write docs\n')

  const done = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo done T1 --json' })
  expect(JSON.parse(done.stdout.delta)).toEqual({
    todo: { id: 'T1', text: 'Run full tests', status: 'done' },
  })

  const list = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo list --json' })
  expect(JSON.parse(list.stdout.delta)).toEqual({
    todos: [
      { id: 'T1', text: 'Run full tests', status: 'done' },
      { id: 'T2', text: 'Write docs', status: 'done' },
    ],
  })
})

test('todo command state is isolated by agent session id', async () => {
  // One shell environment per agent session, as the product composes them.
  const host = await createTodoHost()
  const agentA = await hostlessShell({ host, commands: createCodingCommandRegistry(), agentSessionId: 'agent-a', initialEnv: {} })
  const agentB = await hostlessShell({ host, commands: createCodingCommandRegistry(), agentSessionId: 'agent-b', initialEnv: {} })

  const first = await agentA.exec({ agentSessionId: 'agent-a', script: 'demi todo add "First session"' })
  const second = await agentB.exec({ agentSessionId: 'agent-b', script: 'demi todo add "Second session"' })

  expect(first.stdout.delta).toBe('[ ] T1 First session\n')
  expect(second.stdout.delta).toBe('[ ] T1 Second session\n')
})

test('todo command keeps agent-session storage across shell recreation', async () => {
  let nextShell = 0
  const host = await createTodoHost()
  const env = await hostlessShell({ host, commands: createCodingCommandRegistry(), agentSessionId: 'todo-agent', shellIdFactory: () => `todo-recreated-shell-${++nextShell}`, initialEnv: {} })
  const other = await hostlessShell({ host, commands: createCodingCommandRegistry(), agentSessionId: 'other-agent', shellIdFactory: () => 'other-shell', initialEnv: {} })

  const firstShell = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo add "First shell" --json' })
  expect(firstShell.shellId).toBe('todo-recreated-shell-1')
  expect(await env.disposeShell(firstShell.shellId)).toBe(true)
  const secondShell = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo add "Second shell" --json' })
  const otherAgent = await other.exec({ agentSessionId: 'other-agent', script: 'demi todo add "Other agent" --json' })

  expect(secondShell.shellId).toBe('todo-recreated-shell-2')
  expect(otherAgent.shellId).toBe('other-shell')
  const list = await env.exec({ agentSessionId: 'todo-agent', script: 'demi todo list --json' })
  expect(JSON.parse(list.stdout.delta)).toEqual({
    todos: [
      { id: 'T1', text: 'First shell', status: 'pending' },
      { id: 'T2', text: 'Second shell', status: 'pending' },
    ],
  })
  const otherList = await other.exec({ agentSessionId: 'other-agent', script: 'demi todo list --json' })
  expect(JSON.parse(otherList.stdout.delta)).toEqual({
    todos: [{ id: 'T1', text: 'Other agent', status: 'pending' }],
  })
})

async function createTodoHost(): Promise<LocalHost> {
  const root = await mkdtemp(join(tmpdir(), 'demi-todo-'))
  return new LocalHost(root, { storeRoot: join(root, '.host-store') })
}

async function createTodoEnvironment(shellIdFactory: () => string): Promise<ShellEnvironment> {
  return await hostlessShell({
    host: await createTodoHost(),
    commands: createCodingCommandRegistry(),
    agentSessionId: 'todo-agent',
    shellIdFactory,
    initialEnv: {},
  })
}

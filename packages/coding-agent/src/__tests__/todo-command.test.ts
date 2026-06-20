import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { BashEnvironment } from '@demi/shell'
import { LocalHost } from '@demi/host-local'
import { createCodingCommandRegistry } from '../index'

test('todo command supports add/list/update/done with raw and JSON output', async () => {
  const env = await createTodoEnvironment(() => 'todo-shell')

  const add = await env.exec({ agentSessionId: 'todo-agent', script: 'todo add "Run tests"' })
  expect(add.output.stdoutDelta).toBe('[ ] T1 Run tests\n')

  const addJson = await env.exec({ agentSessionId: 'todo-agent', script: 'todo add "Write docs" --json' })
  expect(JSON.parse(addJson.output.stdoutDelta)).toEqual({
    todo: { id: 'T2', text: 'Write docs', status: 'pending' },
  })

  const rawList = await env.exec({ agentSessionId: 'todo-agent', script: 'todo list' })
  expect(rawList.output.stdoutDelta).toBe('[ ] T1 Run tests\n[ ] T2 Write docs\n')

  const update = await env.exec({ agentSessionId: 'todo-agent', script: 'todo update T1 --text "Run full tests"' })
  expect(update.output.stdoutDelta).toBe('[ ] T1 Run full tests\n')

  const inProgress = await env.exec({ agentSessionId: 'todo-agent', script: 'todo update T1 --status in_progress --json' })
  expect(JSON.parse(inProgress.output.stdoutDelta)).toEqual({
    todo: { id: 'T1', text: 'Run full tests', status: 'in_progress' },
  })

  const doneRaw = await env.exec({ agentSessionId: 'todo-agent', script: 'todo done T2' })
  expect(doneRaw.output.stdoutDelta).toBe('[x] T2 Write docs\n')

  const done = await env.exec({ agentSessionId: 'todo-agent', script: 'todo done T1 --json' })
  expect(JSON.parse(done.output.stdoutDelta)).toEqual({
    todo: { id: 'T1', text: 'Run full tests', status: 'done' },
  })

  const list = await env.exec({ agentSessionId: 'todo-agent', script: 'todo list --json' })
  expect(JSON.parse(list.output.stdoutDelta)).toEqual({
    todos: [
      { id: 'T1', text: 'Run full tests', status: 'done' },
      { id: 'T2', text: 'Write docs', status: 'done' },
    ],
  })
})

test('todo command state is isolated by agent session id', async () => {
  let nextShell = 0
  const env = await createTodoEnvironment(() => `todo-shell-${++nextShell}`)

  const first = await env.exec({ agentSessionId: 'agent-a', script: 'todo add "First session"' })
  const second = await env.exec({ agentSessionId: 'agent-b', script: 'todo add "Second session"' })

  expect(first.output.stdoutDelta).toBe('[ ] T1 First session\n')
  expect(second.output.stdoutDelta).toBe('[ ] T1 Second session\n')
})

test('todo command keeps agent-session storage across shell recreation', async () => {
  let nextShell = 0
  const env = await createTodoEnvironment(() => `todo-recreated-shell-${++nextShell}`)

  const firstShell = await env.exec({ agentSessionId: 'todo-agent', script: 'todo add "First shell" --json' })
  expect(firstShell.shellId).toBe('todo-recreated-shell-1')
  expect(await env.disposeShell(firstShell.shellId)).toBe(true)
  const secondShell = await env.exec({ agentSessionId: 'todo-agent', script: 'todo add "Second shell" --json' })
  const otherAgent = await env.exec({ agentSessionId: 'other-agent', script: 'todo add "Other agent" --json' })

  expect(secondShell.shellId).toBe('todo-recreated-shell-2')
  expect(otherAgent.shellId).toBe('todo-recreated-shell-3')
  const list = await env.exec({ agentSessionId: 'todo-agent', script: 'todo list --json' })
  expect(JSON.parse(list.output.stdoutDelta)).toEqual({
    todos: [
      { id: 'T1', text: 'First shell', status: 'pending' },
      { id: 'T2', text: 'Second shell', status: 'pending' },
    ],
  })
  const otherList = await env.exec({ agentSessionId: 'other-agent', script: 'todo list --json' })
  expect(JSON.parse(otherList.output.stdoutDelta)).toEqual({
    todos: [{ id: 'T1', text: 'Other agent', status: 'pending' }],
  })
})

async function createTodoEnvironment(shellIdFactory: () => string): Promise<BashEnvironment> {
  const root = await mkdtemp(join(tmpdir(), 'demi-todo-'))
  const host = new LocalHost(root, { storeRoot: join(root, '.host-store') })
  return new BashEnvironment({
    host,
    commands: createCodingCommandRegistry(),
    shellIdFactory,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
}

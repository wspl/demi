import { expect, test } from 'bun:test'
import { BashEnvironment } from '@demi/shell'
import { LocalHost } from '@demi/shell/local-host'
import { createCodingCommandRegistry } from '../index'

test('todo command supports add/list/update/done with raw and JSON output', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    commands: createCodingCommandRegistry(),
    shellIdFactory: () => 'todo-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const add = await env.exec({ agentSessionId: 'todo-agent', script: 'todo add "Run tests"' })
  expect(add.output.stdoutDelta).toBe('[ ] T1 Run tests\n')

  const update = await env.exec({ agentSessionId: 'todo-agent', script: 'todo update T1 --text "Run full tests"' })
  expect(update.output.stdoutDelta).toBe('[ ] T1 Run full tests\n')

  const inProgress = await env.exec({ agentSessionId: 'todo-agent', script: 'todo update T1 --status in_progress --json' })
  expect(JSON.parse(inProgress.output.stdoutDelta)).toEqual({
    todo: { id: 'T1', text: 'Run full tests', status: 'in_progress' },
  })

  const done = await env.exec({ agentSessionId: 'todo-agent', script: 'todo done T1 --json' })
  expect(JSON.parse(done.output.stdoutDelta)).toEqual({
    todo: { id: 'T1', text: 'Run full tests', status: 'done' },
  })

  const list = await env.exec({ agentSessionId: 'todo-agent', script: 'todo list --json' })
  expect(JSON.parse(list.output.stdoutDelta)).toEqual({
    todos: [{ id: 'T1', text: 'Run full tests', status: 'done' }],
  })
})

test('todo command state is isolated by agent session id', async () => {
  let nextShell = 0
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    commands: createCodingCommandRegistry(),
    shellIdFactory: () => `todo-shell-${++nextShell}`,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({ agentSessionId: 'agent-a', script: 'todo add "First session"' })
  const second = await env.exec({ agentSessionId: 'agent-b', script: 'todo add "Second session"' })

  expect(first.output.stdoutDelta).toBe('[ ] T1 First session\n')
  expect(second.output.stdoutDelta).toBe('[ ] T1 Second session\n')
})

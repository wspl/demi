import { expect, test } from 'bun:test'
import { BashEnvironment } from '@demi/shell'
import { LocalHost } from '@demi/shell/local-host'
import { createCodingCommandRegistry } from '../index'

test('todo command supports add/list/update/done with raw and JSON output', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    commands: createCodingCommandRegistry(),
    sessionIdFactory: () => 'todo-session',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const add = await env.exec({ script: 'todo add "Run tests"' })
  expect(add.output.stdoutDelta).toBe('[ ] T1 Run tests\n')

  const update = await env.exec({ sessionId: add.sessionId, script: 'todo update T1 --text "Run full tests"' })
  expect(update.output.stdoutDelta).toBe('[ ] T1 Run full tests\n')

  const done = await env.exec({ sessionId: add.sessionId, script: 'todo done T1 --json' })
  expect(JSON.parse(done.output.stdoutDelta)).toEqual({
    todo: { id: 'T1', text: 'Run full tests', status: 'done' },
  })

  const list = await env.exec({ sessionId: add.sessionId, script: 'todo list --json' })
  expect(JSON.parse(list.output.stdoutDelta)).toEqual({
    todos: [{ id: 'T1', text: 'Run full tests', status: 'done' }],
  })
})

test('todo command state is isolated by shell session id', async () => {
  let nextSession = 0
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    commands: createCodingCommandRegistry(),
    sessionIdFactory: () => `todo-session-${++nextSession}`,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({ script: 'todo add "First session"' })
  const second = await env.exec({ script: 'todo add "Second session"' })

  expect(first.output.stdoutDelta).toBe('[ ] T1 First session\n')
  expect(second.output.stdoutDelta).toBe('[ ] T1 Second session\n')
})

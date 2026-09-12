import { expect, test } from 'bun:test'
import { runRegisteredCommand } from '@demicodes/shell'
import { memoryCommandStorage } from '@demicodes/shell/testing'
import { LocalHost } from '@demicodes/runner/testing'
import { createTodoCommand } from '../todo-command'

test('todo reads and mutations reject corrupt state without overwriting it', async () => {
  for (const value of [null, {}, [{ id: 'T1', text: [], status: 'done' }]]) {
    const storage = memoryCommandStorage()
    await storage.writeJson('todos.json', value)
    for (const args of [['list'], ['add', 'New'], ['done', 'T1']]) {
      const invocation = runRegisteredCommand(createTodoCommand(), {
        argv: ['todo', ...args],
        env: {},
        cwd: '/workspace',
        host: new LocalHost('/workspace'),
        storage,
        io: {
          stdout: async () => {},
          stderr: async () => {},
        },
      })
      await expect(invocation).rejects.toThrow()
      expect(await storage.readJson('todos.json')).toEqual(value)
    }
  }
})

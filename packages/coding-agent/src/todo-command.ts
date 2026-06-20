import { z } from 'zod'
import type { CommandSpec, CommandStorage } from '@demi/shell'

const TODO_STORAGE_KEY = 'todos.json'

const TodoStatus = z.enum(['pending', 'in_progress', 'done'])

const TodoItemSchema = z.object({
  id: z.string(),
  text: z.string(),
  status: TodoStatus,
})

const TodoListSchema = z.array(TodoItemSchema)

type TodoItem = z.infer<typeof TodoItemSchema>

export function createTodoCommand(): CommandSpec {
  return {
    name: 'todo',
    summary: 'Manage an agent-session-scoped task list for coding work.',
    subcommands: [
      {
        name: 'list',
        summary: 'List todos for the current agent session.',
        effects: 'read-only; does not modify files or command storage',
        successOutput: 'writes the session todo list as raw text, or JSON matching { todos } when --json is passed',
        failureOutput: 'writes storage or validation errors to stderr and exits non-zero',
        output: {
          json: z.object({ todos: TodoListSchema }),
        },
        examples: ['todo list', 'todo list --json'],
        run: async ({ parsed, io, storage }) => {
          const todos = await readTodos(storage)
          if (parsed.json) {
            await io.stdout(JSON.stringify({ todos }))
          } else if (todos.length === 0) {
            await io.stdout('No todos.\n')
          } else {
            await io.stdout(`${todos.map(formatTodo).join('\n')}\n`)
          }
          return { exitCode: 0 }
        },
      },
      {
        name: 'add',
        summary: 'Add a new todo.',
        effects: 'modifies agent-session-scoped command storage; does not modify files',
        successOutput: 'writes the created todo as raw text, or JSON matching { todo } when --json is passed',
        failureOutput: 'writes validation or storage errors to stderr and exits non-zero',
        input: {
          text: z.string().describe('Todo text'),
        },
        positionals: ['text'],
        output: {
          json: z.object({ todo: TodoItemSchema }),
        },
        examples: ['todo add "Run tests"', 'todo add "Run tests" --json'],
        run: async ({ parsed, io, storage }) => {
          const todos = await readTodos(storage)
          const todo: TodoItem = {
            id: nextTodoId(todos),
            text: String(parsed.values.text),
            status: 'pending',
          }
          todos.push(todo)
          await writeTodos(storage, todos)
          if (parsed.json) await io.stdout(JSON.stringify({ todo }))
          else await io.stdout(`${formatTodo(todo)}\n`)
          return { exitCode: 0 }
        },
      },
      {
        name: 'update',
        summary: 'Update todo text or status.',
        effects: 'modifies agent-session-scoped command storage; does not modify files',
        successOutput: 'writes the updated todo as raw text, or JSON matching { todo } when --json is passed',
        failureOutput: 'writes "Todo not found" or validation/storage errors to stderr and exits non-zero',
        input: {
          id: z.string().describe('Todo id'),
          text: z.string().optional().describe('Replacement text'),
          status: TodoStatus.optional().describe('Replacement status'),
        },
        positionals: ['id'],
        output: {
          json: z.object({ todo: TodoItemSchema }),
        },
        examples: ['todo update T1 --text "Run full test suite"', 'todo update T1 --status in_progress --json'],
        run: async ({ parsed, io, storage }) => {
          const todos = await readTodos(storage)
          const todo = findTodo(todos, String(parsed.values.id))
          if (!todo) {
            await io.stderr(`Todo not found: ${parsed.values.id}\n`)
            return { exitCode: 1 }
          }
          if (parsed.values.text !== undefined) todo.text = String(parsed.values.text)
          if (parsed.values.status !== undefined) todo.status = parsed.values.status as TodoItem['status']
          await writeTodos(storage, todos)
          if (parsed.json) await io.stdout(JSON.stringify({ todo }))
          else await io.stdout(`${formatTodo(todo)}\n`)
          return { exitCode: 0 }
        },
      },
      {
        name: 'done',
        summary: 'Mark a todo as done.',
        effects: 'modifies agent-session-scoped command storage; does not modify files',
        successOutput: 'writes the completed todo as raw text, or JSON matching { todo } when --json is passed',
        failureOutput: 'writes "Todo not found" or validation/storage errors to stderr and exits non-zero',
        input: {
          id: z.string().describe('Todo id'),
        },
        positionals: ['id'],
        output: {
          json: z.object({ todo: TodoItemSchema }),
        },
        examples: ['todo done T1', 'todo done T1 --json'],
        run: async ({ parsed, io, storage }) => {
          const todos = await readTodos(storage)
          const todo = findTodo(todos, String(parsed.values.id))
          if (!todo) {
            await io.stderr(`Todo not found: ${parsed.values.id}\n`)
            return { exitCode: 1 }
          }
          todo.status = 'done'
          await writeTodos(storage, todos)
          if (parsed.json) await io.stdout(JSON.stringify({ todo }))
          else await io.stdout(`${formatTodo(todo)}\n`)
          return { exitCode: 0 }
        },
      },
    ],
  }
}

async function readTodos(storage: CommandStorage): Promise<TodoItem[]> {
  return TodoListSchema.parse((await storage.readJson(TODO_STORAGE_KEY)) ?? [])
}

async function writeTodos(storage: CommandStorage, todos: TodoItem[]): Promise<void> {
  await storage.writeJson(TODO_STORAGE_KEY, todos)
}

function findTodo(todos: TodoItem[], id: string): TodoItem | null {
  return todos.find((todo) => todo.id === id) ?? null
}

function nextTodoId(todos: TodoItem[]): string {
  let max = 0
  for (const todo of todos) {
    const match = /^T(\d+)$/.exec(todo.id)
    if (match) max = Math.max(max, Number(match[1]))
  }
  return `T${max + 1}`
}

function formatTodo(todo: TodoItem): string {
  const marker = todo.status === 'done' ? 'x' : todo.status === 'in_progress' ? '-' : ' '
  return `[${marker}] ${todo.id} ${todo.text}`
}

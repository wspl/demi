import { z } from 'zod'
import type { CommandGroup, CommandStorage } from '@demicodes/shell'

const TODO_STORAGE_KEY = 'todos.json'

const TodoStatus = z.enum(['pending', 'in_progress', 'done'])

const TodoItemSchema = z.object({
  id: z.string(),
  text: z.string(),
  status: TodoStatus,
})

const TodoListSchema = z.array(TodoItemSchema)

type TodoItem = z.infer<typeof TodoItemSchema>

export function createTodoCommand(): CommandGroup {
  return {
    name: 'todo',
    summary: 'Manage an agent-session-scoped task list for coding work.',
    subcommands: [
      {
        name: 'list',
        summary: 'List todos for the current agent session.',
        successOutput: 'writes the session todo list as raw text, or JSON matching { todos } when --json is passed',
        failureOutput: 'writes storage or validation errors to stderr and exits non-zero',
        output: {
          json: z.object({ todos: TodoListSchema }),
        },
        kind: 'rpc',
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
        successOutput: 'writes the created todo as raw text, or JSON matching { todo } when --json is passed',
        failureOutput: 'writes validation or storage errors to stderr and exits non-zero',
        input: {
          text: z.string().describe('Todo text'),
        },
        positionals: ['text'],
        output: {
          json: z.object({ todo: TodoItemSchema }),
        },
        kind: 'rpc',
        run: async ({ parsed, io, storage }) => {
          const todos = await storage.updateJson<TodoItem[]>(TODO_STORAGE_KEY, (current) => {
            const items = TodoListSchema.parse(current ?? [])
            return [...items, {
              id: nextTodoId(items),
              text: String(parsed.values.text),
              status: 'pending',
            }]
          })
          const todo = todos.at(-1)!
          if (parsed.json) await io.stdout(JSON.stringify({ todo }))
          else await io.stdout(`${formatTodo(todo)}\n`)
          return { exitCode: 0 }
        },
      },
      {
        name: 'update',
        summary: 'Update todo text or status.',
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
        kind: 'rpc',
        run: async ({ parsed, io, storage }) => {
          const todo = await updateTodo(storage, String(parsed.values.id), {
            ...(parsed.values.text !== undefined ? { text: String(parsed.values.text) } : {}),
            ...(parsed.values.status !== undefined ? { status: TodoStatus.parse(parsed.values.status) } : {}),
          })
          if (parsed.json) await io.stdout(JSON.stringify({ todo }))
          else await io.stdout(`${formatTodo(todo)}\n`)
          return { exitCode: 0 }
        },
      },
      {
        name: 'done',
        summary: 'Mark a todo as done.',
        successOutput: 'writes the completed todo as raw text, or JSON matching { todo } when --json is passed',
        failureOutput: 'writes "Todo not found" or validation/storage errors to stderr and exits non-zero',
        input: {
          id: z.string().describe('Todo id'),
        },
        positionals: ['id'],
        output: {
          json: z.object({ todo: TodoItemSchema }),
        },
        kind: 'rpc',
        run: async ({ parsed, io, storage }) => {
          const todo = await updateTodo(storage, String(parsed.values.id), { status: 'done' })
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

async function updateTodo(
  storage: CommandStorage,
  id: string,
  patch: Partial<Pick<TodoItem, 'text' | 'status'>>,
): Promise<TodoItem> {
  const todos = await storage.updateJson<TodoItem[]>(TODO_STORAGE_KEY, (current) => {
    const items = TodoListSchema.parse(current ?? [])
    if (!items.some((todo) => todo.id === id)) {
      throw new Error(`Todo not found: ${id}`)
    }
    return items.map((todo) => todo.id === id ? { ...todo, ...patch } : todo)
  })
  return todos.find((todo) => todo.id === id)!
}

function nextTodoId(todos: TodoItem[]): string {
  let max = 0
  for (const todo of todos) {
    const match = /^T(\d+)$/.exec(todo.id)
    if (match)
      max = Math.max(max, Number(match[1]))
  }
  return `T${max + 1}`
}

function formatTodo(todo: TodoItem): string {
  const marker = todo.status === 'done'
    ? 'x'
    : todo.status === 'in_progress' ? '-' : ' '
  return `[${marker}] ${todo.id} ${todo.text}`
}

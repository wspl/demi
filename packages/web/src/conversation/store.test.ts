import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { useConversations } from './store'
import { useProduct } from '../state/product'
import { productStateSchema, type BackendConversation } from '../api/contracts'

const realFetch = globalThis.fetch
let records: BackendConversation[]
let rejectCreate: boolean
let requests: {
  path: string
  body: unknown
}[]
let pinia: ReturnType<typeof createPinia>

function record(id: string): BackendConversation {
  return {
    id,
    title: id,
    pinned: false,
    archived: false,
    readRevision: 0,
    revision: 0,
    unread: false,
    target: { kind: 'cloud' },
    contextVersion: 0,
    providerId: null,
    modelId: null,
    createdAt: '2026-09-09T00:00:00.000Z',
    updatedAt: '2026-09-09T00:00:00.000Z',
    status: 'idle',
  }
}

function snapshot() {
  return productStateSchema.parse({
    user: {
      id: 'user',
      email: 'test@example.test',
      nickname: 'Test',
      role: 'master',
      createdAt: '2026-09-09T00:00:00.000Z',
    },
    mode: 'shared',
    preferences: {
      appearance: {},
      shortcuts: {},
    },
    devices: [],
    workspaces: [],
    providers: [],
    conversations: records,
    cloud: null,
  })
}

beforeEach(async () => {
  pinia = createPinia()
  setActivePinia(pinia)
  records = [record('first'), record('second')]
  rejectCreate = false
  requests = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : null
    requests.push({
      path,
      body,
    })
    if (path === '/api/state') {
      return Response.json(snapshot())
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    if (path === '/api/conversations') {
      if (rejectCreate) {
        return Response.json(
          {
            code: 'unavailable',
            message: 'Not saved',
          },
          { status: 503 },
        )
      }
      const created = record('created')
      records.unshift(created)
      return Response.json({ conversation: created }, { status: 201 })
    }
    if (path === '/api/conversations/batch') {
      const items = body.items as {
        id: string
        patch: Partial<BackendConversation>
      }[]
      return Response.json(
        {
          results: items.map((item) => {
            const current = records.find((record) => record.id === item.id)!
            if (item.id === 'second') {
              return {
                id: item.id,
                conversation: current,
                results: [
                  {
                    field: 'archived',
                    status: 'failed',
                    code: 'busy',
                    message: 'Turn is running',
                  },
                ],
              }
            }
            Object.assign(current, item.patch)
            return {
              id: item.id,
              conversation: current,
              results: Object.keys(item.patch).map((field) => ({
                field,
                status: 'applied',
              })),
            }
          }),
        },
        { status: 207 },
      )
    }
    if (path.endsWith('/read')) {
      return new Response(null, { status: 204 })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  useConversations()
  await useProduct().start()
  await nextTick()
})

afterEach(() => {
  useConversations().stopAll()
  useProduct().stop()
  disposePinia(pinia)
  globalThis.fetch = realFetch
})

test('a failed create does not insert a fabricated conversation', async () => {
  const store = useConversations()
  rejectCreate = true
  expect(await store.create()).toBeNull()
  expect(store.items.map((item) => item.id)).toEqual(['first', 'second'])
  expect(store.notice).toBe('Not saved')
})

test('created IDs and sidebar order come from the server', async () => {
  const store = useConversations()
  expect(await store.create()).toBe('created')
  expect(store.items.map((item) => item.id)).toEqual(['created', 'first', 'second'])
})

test('snapshot refresh preserves live transcript and unsent draft', async () => {
  const store = useConversations()
  const current = store.items[0]!
  current.draft = 'Still editing'
  current.phase = 'running'
  current.blocks = [
    {
      type: 'extension_state_snapshot',
      id: 'block',
      createdAt: '2026-09-09T00:00:00.000Z',
      extensionName: 'example',
      state: { value: 1 },
    },
  ]
  records[0]!.title = 'Server title'
  records.reverse()
  await useProduct().refresh()
  await nextTick()
  expect(store.items[1]).toBe(current)
  expect(current.title).toBe('Server title')
  expect(current.draft).toBe('Still editing')
  expect(current.blocks[0]?.id).toBe('block')
  expect(current.phase).toBe('running')
})

test('batch partial failure applies only the successful server records', async () => {
  const store = useConversations()
  expect(await store.archive(['first', 'second'])).toBe(false)
  expect(store.items.find((item) => item.id === 'first')?.archived).toBe(true)
  expect(store.items.find((item) => item.id === 'second')?.archived).toBe(false)
  expect(store.notice).toContain('second: Turn is running')
})

test('read acknowledgements use the observed revision and wait for history', async () => {
  const original = globalThis.document
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: { visibilityState: 'visible' },
  })
  try {
    const store = useConversations()
    const current = store.items[0]!
    current.unread = true
    current.revision = 7
    await store.markRead(current.id)
    expect(requests.some((request) => request.path.endsWith('/read'))).toBe(false)
    current.load = 'ready'
    await store.markRead(current.id)
    expect(requests.at(-1)?.body).toEqual({ revision: 7 })
    expect(current.unread).toBe(false)
  } finally {
    Object.defineProperty(globalThis, 'document', {
      configurable: true,
      value: original,
    })
  }
})

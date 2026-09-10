import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { useConversations } from './store'
import { useProduct } from '../state/product'
import { productStateSchema, type BackendConversation } from '../api/contracts'

const realFetch = globalThis.fetch
let records: BackendConversation[]
let rejectCreate: boolean
let rejectFork: boolean
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
  rejectFork = false
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
    if (path === '/api/conversations/first/fork') {
      if (rejectFork) {
        return Response.json({ code: 'unavailable', message: 'Fork unavailable' }, { status: 503 })
      }
      const created = records.find((item) => item.id === body.id) ?? {
        ...record(body.id), title: 'first (Fork)', providerId: 'stub', modelId: 'model',
        target: { kind: 'cloud' as const, path: '/home/demi/sessions/first' },
      }
      if (!records.includes(created)) records.unshift(created)
      return Response.json({
        conversation: created,
        model: {
          providerId: 'stub', serviceTierId: 'priority',
          thinking: { type: 'effort', effort: 'high', summary: null },
          model: { id: 'model', name: 'Model', contextWindow: 1000,
            outputLimit: null, inputLimit: null, acceptedExtensions: [], thinking: [] },
        },
      }, { status: 201 })
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
      const created = records.find((item) => item.id === body.id) ?? record(body.id)
      if (!records.includes(created)) {
        records.unshift(created)
      }
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

test('new conversation is local and does not depend on the server', async () => {
  const store = useConversations()
  rejectCreate = true
  const id = await store.create()
  expect(id).toMatch(/^[0-9a-f-]{36}$/)
  expect(store.items.map((item) => item.id)).toEqual([id!, 'first', 'second'])
  expect(store.items[0]?.persistence).toBe('draft')
  expect(store.items[0]?.load).toBe('ready')
  expect(requests.some((request) => request.path === '/api/conversations')).toBe(false)
})

test('repeated new reuses the active empty draft and polling preserves it', async () => {
  const store = useConversations()
  const id = await store.create()
  await store.activate(id)
  expect(await store.create()).toBe(id)
  await useProduct().refresh()
  await nextTick()
  expect(store.items[0]?.id).toBe(id)
  store.items[0]!.draft = 'Keep this draft'
  expect(await store.create()).not.toBe(id)
})

test('first send failure preserves the conversation and retries its UUID and message', async () => {
  const store = useConversations()
  await store.create()
  const conversation = store.items[0]!
  conversation.draft = 'First message'
  rejectCreate = true
  await store.send(conversation)
  expect(conversation.persistence).toBe('pending')
  expect(conversation.pendingSend?.text).toBe('First message')
  expect(conversation.pendingSend?.error).toBe('Not saved')
  expect(conversation.draft).toBe('')
  const messageId = conversation.pendingSend!.id
  await useProduct().refresh()
  await nextTick()
  expect(store.items[0]?.id).toBe(conversation.id)
  await store.send(conversation)
  expect(conversation.pendingSend?.id).toBe(messageId)
  expect(requests.filter((request) => request.path === '/api/conversations').map((request) => request.body)).toEqual([
    { id: conversation.id },
    { id: conversation.id },
  ])
})

test('workspace attachments stay local until first send creates the conversation', async () => {
  const store = useConversations()
  store.create()
  const conversation = store.items[0]!
  store.addFiles(conversation, [new File(['Local notes'], 'notes.txt', { type: 'text/plain' })], ['png', 'pdf'])
  await nextTick()
  expect(conversation.files[0]).toMatchObject({
    kind: 'file',
    destination: 'workspace',
    phase: 'staged',
    upload: null,
  })
  expect(requests.some((request) => request.path === '/api/conversations')).toBe(false)
  rejectCreate = true
  await store.send(conversation)
  expect(conversation.pendingSend?.fileIds).toEqual([conversation.files[0]!.id])
  expect(conversation.files[0]).toMatchObject({ phase: 'staged', upload: null })
})

test('leaving an empty draft discards it without removing a draft with input', async () => {
  const store = useConversations()
  const empty = store.create()
  await store.activate(empty)
  await store.activate(null)
  expect(store.items.some((item) => item.id === empty)).toBe(false)
  const retained = store.create()
  await store.activate(retained)
  store.items.find((item) => item.id === retained)!.draft = 'Keep this'
  await store.activate(null)
  expect(store.items.some((item) => item.id === retained)).toBe(true)
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

test('Fork preserves the source draft and opens an independent empty composer with inherited model settings', async () => {
  const store = useConversations()
  const source = store.items.find((item) => item.id === 'first')!
  source.draft = 'Keep this unsent message'
  source.phase = 'running'
  const request = { id: crypto.randomUUID(), blockId: 'completed-answer' }
  expect(await store.fork(source.id, request)).toBe(request.id)
  const branch = store.items.find((item) => item.id === request.id)!
  expect(branch.title).toBe('first (Fork)')
  expect(branch.draft).toBe('')
  expect(branch.files).toEqual([])
  expect(branch.model).toEqual({ providerId: 'stub', modelId: 'model', thinkingEffort: 'high', serviceTierId: 'priority' })
  expect(branch.target).toEqual({ kind: 'cloud', path: '/home/demi/sessions/first' })
  expect(source.draft).toBe('Keep this unsent message')
  expect(source.phase).toBe('running')
})

test('failed Fork creates no local conversation; retry forwards the same destination ID', async () => {
  const store = useConversations()
  const request = { id: crypto.randomUUID(), blockId: 'completed-answer' }
  rejectFork = true
  await expect(store.fork('first', request)).rejects.toThrow('Fork unavailable')
  expect(store.items.some((item) => item.id === request.id)).toBe(false)
  rejectFork = false
  await store.fork('first', request)
  expect(requests.filter((item) => item.path.endsWith('/fork')).map((item) => item.body))
    .toEqual([request, request])
  expect(store.items.filter((item) => item.id === request.id)).toHaveLength(1)
})

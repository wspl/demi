import { afterEach, beforeEach, expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { createApp, defineComponent, nextTick } from 'vue'
import { createMemoryHistory, createRouter } from 'vue-router'
import type { WorkspaceDto } from '../api/generated/web-api'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { useConversationNavigation } from './navigation'
import { useConversations } from './store'

// Cost: stores, a memory router and a played channel; milliseconds.

const realFetch = globalThis.fetch
const PROJECT: WorkspaceDto = {
  id: 'project-1',
  deviceId: 'cloud',
  path: '/home/demi/workspaces/project-1',
  name: 'Notes',
  createdAt: '2026-10-06T00:00:00.000Z',
}

let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
let requests: { method: string; path: string; body: unknown }[]

beforeEach(async () => {
  pinia = createPinia()
  setActivePinia(pinia)
  requests = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    const method = init?.method ?? 'GET'
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : null
    requests.push({ method, path, body })
    if (path === '/api/workspaces' && method === 'POST') {
      // The backend answers, and its channel brings the new project.
      queueMicrotask(() => channels.last().send({ type: 'snapshot', state: productState({ workspaces: [PROJECT] }) }))
      return Response.json({ workspace: PROJECT }, { status: 201 })
    }
    if (path === '/api/conversations' && method === 'POST') {
      return Response.json({ code: 'internal_error', message: 'Not saved' }, { status: 503 })
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    throw new Error(`Unexpected request: ${method} ${path}`)
  }) as typeof fetch
  channels = playChannels()
  useConversations()
  useProduct().start()
  channels.last().connect(productState())
  await nextTick()
})

afterEach(() => {
  useConversations().stopAll()
  usePreferences().stop()
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
})

test('creating a project opens a new conversation draft in it, which only the first send makes a conversation', async () => {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/chat/:id?', component: defineComponent({ render: () => null }) }],
  })
  const app = createApp({})
  app.use(pinia)
  app.use(router)
  const navigation = app.runWithContext(() => useConversationNavigation())
  const conversations = useConversations()

  await navigation.createProject({ kind: 'cloud', name: 'Notes' })
  await waitFor(() => router.currentRoute.value.path.startsWith('/chat/'), () => router.currentRoute.value.fullPath)
  const conversation = conversations.items.find((item) => item.id === router.currentRoute.value.params.id)!
  expect(conversation.target).toEqual({ kind: 'workspace', workspaceId: PROJECT.id })
  expect(conversation.persistence).toBe('draft')
  expect(requests.some((request) => request.path === '/api/conversations')).toBe(false)

  conversation.draft = 'First message'
  await conversations.send(conversation)
  // The record starts in the project, with the rest of what the draft holds.
  expect(requests.filter((request) => request.path === '/api/conversations')).toEqual([
    {
      method: 'POST',
      path: '/api/conversations',
      body: {
        id: conversation.id,
        title: conversation.title,
        pinned: false,
        target: { kind: 'workspace', workspaceId: PROJECT.id },
      },
    },
  ])
})

test('New on the empty draft already shown gives it back with the focus asked for its composer', async () => {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/chat/:id?', component: defineComponent({ render: () => null }) }],
  })
  const app = createApp({})
  app.use(pinia)
  app.use(router)
  const navigation = app.runWithContext(() => useConversationNavigation())
  const conversations = useConversations()

  navigation.create(null)
  await waitFor(() => router.currentRoute.value.path.startsWith('/chat/'), () => router.currentRoute.value.fullPath)
  const draft = router.currentRoute.value.params.id
  // A new draft shows its composer anew, which takes the focus as it shows.
  expect(conversations.composerFocusRequests).toBe(0)
  navigation.create(null)
  await nextTick()
  expect(router.currentRoute.value.params.id).toBe(draft)
  expect(conversations.composerFocusRequests).toBe(1)
})

test('a new conversation opened from the list shows on an entry that says so, as New\'s does', async () => {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/chat/:id?', component: defineComponent({ render: () => null }) }],
  })
  const app = createApp({})
  app.use(pinia)
  app.use(router)
  const navigation = app.runWithContext(() => useConversationNavigation())
  const id = useConversations().create(null)
  useConversations().items.find((item) => item.id === id)!.draft = 'Fix the login bug'
  await router.push('/chat')
  expect(navigation.showsNewConversation()).toBe(false)
  navigation.open(id)
  await waitFor(() => router.currentRoute.value.params.id === id, () => router.currentRoute.value.fullPath)
  expect(navigation.showsNewConversation()).toBe(true)
})

test('a reload of a new conversation, which has no record yet, opens a new conversation again, and an unknown address stays not found', async () => {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/chat/:id?', component: defineComponent({ render: () => null }) }],
  })
  const app = createApp({})
  app.use(pinia)
  app.use(router)
  const navigation = app.runWithContext(() => useConversationNavigation())
  navigation.create(null)
  await waitFor(() => router.currentRoute.value.path.startsWith('/chat/'), () => router.currentRoute.value.fullPath)
  const shown = router.currentRoute.value.params.id

  // The reload: a new page, whose stores know nothing of the draft, on the
  // same history entry, which the web browser keeps.
  useConversations().stopAll()
  useProduct().stop()
  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  useConversations()
  useProduct().start()
  channels.last().connect(productState())
  const reloaded = createApp({})
  reloaded.use(pinia)
  reloaded.use(router)
  const again = reloaded.runWithContext(() => useConversationNavigation())
  expect(useConversations().items.some((item) => item.id === shown)).toBe(false)
  expect(again.reopenNew()).toBe(true)
  await waitFor(() => router.currentRoute.value.params.id !== shown, () => router.currentRoute.value.fullPath)
  const opened = useConversations().items.find((item) => item.id === router.currentRoute.value.params.id)
  expect(opened?.persistence).toBe('draft')

  // An address that never showed a new conversation, as a link to a
  // deleted one, is not taken for one.
  await router.push('/chat/00000000-0000-4000-8000-00000000dead')
  expect(again.reopenNew()).toBe(false)
  expect(router.currentRoute.value.params.id).toBe('00000000-0000-4000-8000-00000000dead')
})

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
  expect(requests.filter((request) => request.path === '/api/conversations')).toEqual([
    { method: 'POST', path: '/api/conversations', body: { id: conversation.id } },
  ])
})

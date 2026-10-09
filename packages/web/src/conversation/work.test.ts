import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { useSession } from '../auth/session'
import { identitySchema } from '../api/generated/web-api'
import { readLocalState } from '../state/local'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { conversationSummary, productState } from '../__tests__/product-state'
import type { PanelTab } from '@demicodes/web-ui/agent/panel-tabs'
import { useConversations } from './store'
import { useWorkPanel } from './work'

const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
const saved = new Map<string, string>()
let pinia: ReturnType<typeof createPinia>

beforeEach(() => {
  saved.clear()
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => saved.get(key) ?? null,
      setItem: (key: string, value: string) => saved.set(key, value),
    },
  })
  pinia = createPinia()
  setActivePinia(pinia)
})

afterEach(() => {
  disposePinia(pinia)
  if (storageDescriptor) {
    Object.defineProperty(globalThis, 'localStorage', storageDescriptor)
  } else {
    Reflect.deleteProperty(globalThis, 'localStorage')
  }
})

function signIn(id: string): void {
  const { user } = identitySchema.parse({ user: {
    id,
    email: `${id}@example.test`,
    nickname: id,
    role: 'master',
    createdAt: '2026-09-18T00:00:00.000Z',
  } })
  useSession().current = { status: 'signedIn', user }
}

test('panel open and closed choices survive reload with the panel share in the same preferences', async () => {
  signIn('one')
  const work = useWorkPanel()
  expect(work.stateFor('a').open).toBe(false)
  useResources().asideShare = 0.5
  work.setOpen(work.stateFor('a'), true)
  await nextTick()
  expect(readLocalState('one')).toMatchObject({ asideShare: 0.5, workPanelOpen: { a: true } })

  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  signIn('one')
  const restored = useWorkPanel()
  expect(restored.stateFor('a').open).toBe(true)
  expect(restored.stateFor('b').open).toBe(false)
  expect(useResources().asideShare).toBe(0.5)
  restored.setOpen(restored.stateFor('a'), false)
  await nextTick()
  expect(readLocalState('one').workPanelOpen?.a).toBe(false)
})

test('the changes of a request open in the Change view with the panel, only while the changes plugin is on', async () => {
  signIn('one')
  const product = useProduct()
  const changes = { id: 'changes', name: 'Changes', description: 'Shows changes.', enabled: false, packages: [] }
  product.snapshot = productState({ plugins: [changes] })
  const work = useWorkPanel()
  const state = work.stateFor('a')
  const edit = { node: null, request: 'user', file: 'index.ts', edit: { call: 'call', path: 'index.ts', segment: 0 } }
  // Off, nothing opens the edit: the transcript's pills are no controls.
  expect(work.canOpen('edit')).toBe(false)
  work.openIn('a', { intent: 'edit', payload: edit })
  expect(state.open).toBe(false)

  product.snapshot = productState({ plugins: [{ ...changes, enabled: true }] })
  expect(work.canOpen('edit')).toBe(true)
  work.openIn('a', { intent: 'edit', payload: edit })
  await nextTick()
  expect(readLocalState('one').workPanelOpen?.a).toBe(true)
  // The edit takes the selection in the Change view's pinned tab; nothing is saved over a panel that was never read.
  expect(state.panel.history).toEqual(['change'])
  expect(state.pinned.change).toMatchObject({ mode: 'conversation', request: { request: 'user', file: 'index.ts' } })
  signIn('two')
  await nextTick()
  expect(state.open).toBe(false)
  signIn('one')
  await nextTick()
  expect(state.open).toBe(true)
})

/**
 * Answers the work panel's routes as the backend does, from `stored`,
 * counting its reads and keeping each change it was sent.
 */
function stubPanelRoutes(stored: { revision: number; tabs: PanelTab[] }) {
  const route = { stored, reads: 0, sent: [] as { method: string; path: string; body: unknown }[] }
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = new URL(String(input), 'http://page.test').pathname
    const method = init?.method ?? 'GET'
    if (method === 'GET') {
      route.reads += 1
      return Response.json(route.stored)
    }
    route.sent.push({ method, path, body: init?.body ? JSON.parse(String(init.body)) : null })
    route.stored = { ...route.stored, revision: route.stored.revision + 1 }
    return Response.json({ revision: route.stored.revision, changed: true })
  }) as typeof fetch
  return route
}

const fetchDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'fetch')

afterEach(() => {
  if (fetchDescriptor) {
    Object.defineProperty(globalThis, 'fetch', fetchDescriptor)
  }
})

async function settled(): Promise<void> {
  for (let turn = 0; turn < 20; turn++) {
    await Promise.resolve()
  }
}

test('a new conversation\'s panel reads and sends nothing until its first send creates the record', async () => {
  signIn('one')
  const route = stubPanelRoutes({ revision: 0, tabs: [] })
  const conversations = useConversations()
  const id = conversations.create()
  const work = useWorkPanel()
  work.load(id)
  const tab = work.add(id, 'page', { url: 'https://example.test/' })
  await settled()
  // The tab shows at once, selected, and nothing reached the backend.
  expect(work.stateFor(id).panel).toEqual({ history: [tab], tabs: [{ id: tab, kind: 'page', data: { url: 'https://example.test/' } }] })
  expect(work.recorded(id)).toBe(false)
  expect(route.reads).toBe(0)
  expect(route.sent).toEqual([])

  // What the first send's record does: the panel is read, and what the user opened before is sent.
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  work.load(id)
  await settled()
  expect(route.reads).toBeGreaterThan(0)
  expect(route.sent).toEqual([{
    method: 'POST',
    path: `/api/conversations/${id}/panel/changes`,
    body: { changes: [{ op: 'create', id: tab, kind: 'page', data: { url: 'https://example.test/' } }] },
  }])
})

test('the selection is this page\'s own, and a higher revision in the summary reads the panel again', async () => {
  signIn('one')
  const route = stubPanelRoutes({ revision: 1, tabs: [{ id: 'p1', kind: 'page', data: { url: 'https://a.test/' } }] })
  const conversations = useConversations()
  const id = conversations.create()
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  const product = useProduct()
  const summary = (panelRevision: number) => productState({ conversations: [conversationSummary(id, '', { panelRevision })] })
  product.snapshot = summary(1)
  const work = useWorkPanel()
  work.load(id)
  await settled()
  expect(work.stateFor(id).panel.tabs.map((tab) => tab.id)).toEqual(['p1'])
  work.select(id, 'p1')
  await nextTick()
  expect(readLocalState('one').workPanelHistory).toEqual({ [id]: ['p1'] })

  // Another page added a tab: its summary's revision rose.
  route.stored = { revision: 2, tabs: [...route.stored.tabs, { id: 'p2', kind: 'page', data: { url: 'https://b.test/' } }] }
  product.snapshot = summary(2)
  await settled()
  expect(work.stateFor(id).panel).toEqual({
    history: ['p1'],
    tabs: [
      { id: 'p1', kind: 'page', data: { url: 'https://a.test/' } },
      { id: 'p2', kind: 'page', data: { url: 'https://b.test/' } },
    ],
  })

  // Closing the tab the page went to last leaves the one before it, which a reload shows again.
  work.select(id, 'p2')
  work.closeTabs(id, ['p2'])
  await nextTick()
  expect(readLocalState('one').workPanelHistory).toEqual({ [id]: ['p1'] })
})

test('a tab its kind asks to show opens the closed panel and is selected once, and a reload does not apply it again', async () => {
  signIn('one')
  const tabs = (shows?: number): PanelTab[] => [
    { id: 'b1', kind: 'browser', data: { url: 'https://a.test/', tab: 't1', ...(shows ? { shows } : {}) } },
    { id: 'b2', kind: 'browser', data: { url: 'https://b.test/', tab: 't2' } },
  ]
  const route = stubPanelRoutes({ revision: 1, tabs: tabs() })
  const conversations = useConversations()
  const id = conversations.create()
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  const product = useProduct()
  const summary = (panelRevision: number) => productState({
    conversations: [conversationSummary(id, '', { panelRevision })],
    plugins: [{ id: 'browser', name: 'Browser', description: 'A browser.', enabled: true, packages: [] }],
  })
  product.snapshot = summary(1)
  const work = useWorkPanel()
  work.load(id)
  await settled()
  work.select(id, 'b2')
  expect(work.stateFor(id).open).toBe(false)

  // The agent shows t1 while the panel is closed: the plugin raises the tab's count.
  route.stored = { revision: 2, tabs: tabs(1) }
  product.snapshot = summary(2)
  await settled()
  expect([work.stateFor(id).open, work.stateFor(id).history.at(-1)]).toEqual([true, 'b1'])
  // The user's own selection after it stands, however often the panel is read again.
  work.select(id, 'b2')
  route.stored = { revision: 3, tabs: tabs(1) }
  product.snapshot = summary(3)
  await settled()
  expect(work.stateFor(id).history.at(-1)).toBe('b2')
  expect(readLocalState('one').workPanelShown).toEqual({ [id]: { b1: 1 } })

  // A reloaded page knows what it applied.
  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  signIn('one')
  const reloadedRoute = stubPanelRoutes({ revision: 3, tabs: tabs(1) })
  useConversations().items.push(...conversations.items)
  useProduct().snapshot = summary(3)
  const reloaded = useWorkPanel()
  reloaded.setOpen(reloaded.stateFor(id), false)
  reloaded.load(id)
  await settled()
  expect([reloaded.stateFor(id).open, reloaded.stateFor(id).history.at(-1)]).toEqual([false, 'b2'])
  // A new showing shows it again.
  reloadedRoute.stored = { revision: 4, tabs: tabs(2) }
  useProduct().snapshot = summary(4)
  await settled()
  expect([reloaded.stateFor(id).open, reloaded.stateFor(id).history.at(-1)]).toEqual([true, 'b1'])
})

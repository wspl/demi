import { afterEach, beforeEach, expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { useSession } from '../auth/session'
import { identitySchema } from '../api/generated/web-api'
import { readLocalState } from '../state/local'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { productState } from '../__tests__/product-state'
import type { PanelState } from '@demicodes/web-ui/agent/panel-tabs'
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

test('a retained edit opens in the Change view with the panel, only while the changes plugin is on', async () => {
  signIn('one')
  const product = useProduct()
  const changes = { id: 'changes', name: 'Changes', description: 'Shows changes.', enabled: false, packages: [] }
  product.snapshot = productState({ plugins: [changes] })
  const work = useWorkPanel()
  const state = work.stateFor('a')
  const edit = {
    commandId: 'call',
    file: { path: 'index.ts', kind: 'modified' as const, added: 1, removed: 1, edits: [{}] },
  }
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
  expect(state.panel.selection).toBe('change')
  expect(state.pinned.change).toMatchObject({ mode: 'conversation', call: { commandId: 'call' } })
  signIn('two')
  await nextTick()
  expect(state.open).toBe(false)
  signIn('one')
  await nextTick()
  expect(state.open).toBe(true)
})

/**
 * Answers the work panel's route from `stored`, counting its reads and
 * keeping each save; a read waits for `hold` while one is set.
 */
function stubPanelRoute(stored: PanelState) {
  const route = {
    stored,
    reads: 0,
    puts: [] as unknown[],
    hold: null as ReturnType<typeof deferred<void>> | null,
  }
  globalThis.fetch = (async (_input: RequestInfo | URL, init?: RequestInit) => {
    if (init?.method === 'PUT') {
      const body = JSON.parse(String(init.body))
      route.puts.push(body)
      route.stored = body
      return new Response(null, { status: 204 })
    }
    route.reads += 1
    const answer = JSON.stringify(route.stored)
    await route.hold?.promise
    return new Response(answer, { status: 200, headers: { 'content-type': 'application/json' } })
  }) as typeof fetch
  return route
}

const fetchDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'fetch')

afterEach(() => {
  if (fetchDescriptor) {
    Object.defineProperty(globalThis, 'fetch', fetchDescriptor)
  }
})

test('the saved panel is read once, keeps what the user did before it arrived, and saves leave in order', async () => {
  signIn('one')
  const route = stubPanelRoute({ selection: 'change', tabs: [{ id: 'tab-1', kind: 'browser', data: { url: 'about:blank' } }] })
  const conversations = useConversations()
  const id = conversations.create()
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  const work = useWorkPanel()
  // The read is on its way while the user already acts: what they did stays.
  route.hold = deferred<void>()
  const reading = work.load(id)
  work.add(id, 'page', { url: 'https://example.test/', expose: null }, { select: true })
  route.hold.resolve()
  await reading
  expect(work.stateFor(id).panel.tabs.map((tab) => tab.kind)).toEqual(['browser', 'page'])

  // The panel is not read again: the page's state is the newest there is.
  work.update(id, 'tab-1', { url: 'about:blank', tab: 't_bound' })
  await work.load(id)
  expect(route.reads).toBe(1)
  expect(work.stateFor(id).panel.tabs[0]!.data).toEqual({ url: 'about:blank', tab: 't_bound' })

  // Changes made while a save is out leave afterwards, the latest last.
  work.update(id, 'tab-1', { url: 'https://example.test/', tab: 't_bound' })
  work.select(id, 'tab-1')
  for (let turn = 0; turn < 10; turn++) {
    await Promise.resolve()
  }
  expect(route.puts.at(-1)).toMatchObject({
    selection: 'tab-1',
    tabs: [{ id: 'tab-1', kind: 'browser', data: { url: 'https://example.test/', tab: 't_bound' } }, { kind: 'page' }],
  })
  expect(route.stored).toEqual(work.stateFor(id).panel)
})

test('a new conversation\'s panel reads and saves nothing until its first send creates the record', async () => {
  signIn('one')
  const route = stubPanelRoute({ selection: null, tabs: [] })
  const conversations = useConversations()
  const id = conversations.create()
  const work = useWorkPanel()
  await work.load(id)
  work.add(id, 'page', { url: 'https://example.test/', expose: null }, { select: true })
  await Promise.resolve()
  expect(work.recorded(id)).toBe(false)
  expect(route.reads).toBe(0)
  expect(route.puts).toEqual([])

  // What the first send's record does: the panel is read, and what the user opened before stays and is saved.
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  expect(work.recorded(id)).toBe(true)
  await work.load(id)
  for (let turn = 0; turn < 10; turn++) {
    await Promise.resolve()
  }
  expect(route.reads).toBe(1)
  expect(route.puts.at(-1)).toMatchObject({ tabs: [{ kind: 'page' }] })
})

import { afterEach, beforeEach, expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { useSession } from '../auth/session'
import { identitySchema } from '../api/contracts'
import { readLocalState } from '../state/local'
import { useResources } from '../state/resources'
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

test('opening a retained edit persists the panel and account changes isolate choices', async () => {
  signIn('one')
  const work = useWorkPanel()
  const state = work.stateFor('a')
  work.selectEdit('a', {
    commandId: 'call',
    file: { path: 'index.ts', kind: 'modified', added: 1, removed: 1, edits: [{ kept: true }] },
  })
  await nextTick()
  expect(readLocalState('one').workPanelOpen?.a).toBe(true)
  // The edit takes the selection; nothing is saved over a panel that was never read.
  expect(state.panel.selection).toBe('change')
  signIn('two')
  await nextTick()
  expect(state.open).toBe(false)
  signIn('one')
  await nextTick()
  expect(state.open).toBe(true)
})

test('a read of the saved panel never takes back what the page changed meanwhile, and saves leave in order', async () => {
  signIn('one')
  const fetchDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'fetch')
  let stored = { selection: 'change', tabs: [{ id: 'tab-1', kind: 'browser', data: { url: 'about:blank' } }] }
  const puts: unknown[] = []
  let heldRead: ReturnType<typeof deferred<void>> | null = null
  globalThis.fetch = (async (_input: RequestInfo | URL, init?: RequestInit) => {
    if (init?.method === 'PUT') {
      const body = JSON.parse(String(init.body))
      puts.push(body)
      stored = body
      return new Response(null, { status: 204 })
    }
    const answer = JSON.stringify(stored)
    await heldRead?.promise
    return new Response(answer, { status: 200, headers: { 'content-type': 'application/json' } })
  }) as typeof fetch
  try {
    const work = useWorkPanel()
    await work.load('a')
    expect(work.stateFor('a').panel).toEqual(stored)

    // A read is on its way while the tab's content binds its browser tab: the read is stale.
    heldRead = deferred<void>()
    const reading = work.load('a')
    work.update('a', 'tab-1', { url: 'about:blank', tab: 't_bound' })
    heldRead.resolve()
    await reading
    expect(work.stateFor('a').panel.tabs[0]!.data).toEqual({ url: 'about:blank', tab: 't_bound' })

    // Changes made while a save is out leave afterwards, the latest last.
    work.update('a', 'tab-1', { url: 'https://example.test/', tab: 't_bound' })
    work.select('a', 'tab-1')
    for (let turn = 0; turn < 10; turn++) {
      await Promise.resolve()
    }
    expect(puts.at(-1)).toEqual({
      selection: 'tab-1',
      tabs: [{ id: 'tab-1', kind: 'browser', data: { url: 'https://example.test/', tab: 't_bound' } }],
    })
    expect(stored).toEqual(work.stateFor('a').panel)
  } finally {
    if (fetchDescriptor) {
      Object.defineProperty(globalThis, 'fetch', fetchDescriptor)
    }
  }
})

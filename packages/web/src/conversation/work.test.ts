import { afterEach, beforeEach, expect, test } from 'bun:test'
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
  work.selectEdit(state, {
    commandId: 'call',
    file: { path: 'index.ts', kind: 'modified', added: 1, removed: 1, edits: [{ kept: true }] },
  })
  await nextTick()
  expect(readLocalState('one').workPanelOpen?.a).toBe(true)
  signIn('two')
  await nextTick()
  expect(state.open).toBe(false)
  signIn('one')
  await nextTick()
  expect(state.open).toBe(true)
})

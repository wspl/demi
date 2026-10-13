import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { identitySchema } from '../api/generated/web-api'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { useSession } from './session'
import { usePasskeyOffer } from './passkey-offer'

// Cost: no I/O, stubbed storages and a played sync channel; a few milliseconds.
const GLOBALS = ['localStorage', 'sessionStorage', 'PublicKeyCredential'] as const
const saved = GLOBALS.map((name) => Object.getOwnPropertyDescriptor(globalThis, name))
/** What the browser keeps on this device across page loads, and in this tab. */
const local = new Map<string, string>()
const tab = new Map<string, string>()
let pinia: ReturnType<typeof createPinia> | null = null
let channels: ReturnType<typeof playChannels>

function storage(items: Map<string, string>) {
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => items.set(key, value),
    removeItem: (key: string) => items.delete(key),
  }
}

beforeEach(() => {
  local.clear()
  tab.clear()
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage(local) })
  Object.defineProperty(globalThis, 'sessionStorage', { configurable: true, value: storage(tab) })
  Object.defineProperty(globalThis, 'PublicKeyCredential', { configurable: true, value: class {} })
  channels = playChannels()
  loadPage()
})

afterEach(() => {
  endPage()
  channels.restore()
  GLOBALS.forEach((name, index) => {
    const descriptor = saved[index]
    if (descriptor) {
      Object.defineProperty(globalThis, name, descriptor)
    } else {
      Reflect.deleteProperty(globalThis, name)
    }
  })
})

/** The page ends: its stores and channel go. */
function endPage(): void {
  if (pinia) {
    useProduct().stop()
    disposePinia(pinia)
    pinia = null
  }
}

/** A new page load: its stores are new, the browser's storages are not. */
function loadPage(): void {
  endPage()
  pinia = createPinia()
  setActivePinia(pinia)
}

/** The page signs in as `id` and its account loads, on a server that offers passkeys. */
function accountLoads(id: string): void {
  const { user } = identitySchema.parse({
    user: { id, email: `${id}@example.test`, nickname: id, role: 'master', createdAt: '2026-09-18T00:00:00.000Z', disabled: false, choosesPassword: false },
  })
  useSession().current = { status: 'signedIn', user }
  useProduct().start()
  channels.last().connect(productState({ passkeyRpId: 'demi.example' }))
}

test('the phone a handoff landed on is offered a passkey once, whichever way it answers', async () => {
  // The handoff page asks, then loads the conversation: a new page.
  loadPage()
  usePasskeyOffer().request()
  loadPage()
  const offer = usePasskeyOffer()
  expect(offer.open).toBe(false)
  accountLoads('ana')
  await nextTick()
  expect(offer.open).toBe(true)

  // Pressing either button answers it; the card goes when its work is done.
  offer.answered()
  offer.close()
  expect(offer.open).toBe(false)
  // The browser's preferences are written a tick after they change.
  await nextTick()

  // Not on this browser's next landing, and not on the next page of the tab.
  loadPage()
  usePasskeyOffer().request()
  loadPage()
  accountLoads('ana')
  await nextTick()
  expect(usePasskeyOffer().open).toBe(false)

  // Another account on the same browser has not been asked.
  loadPage()
  usePasskeyOffer().request()
  loadPage()
  accountLoads('bo')
  await nextTick()
  expect(usePasskeyOffer().open).toBe(true)
})

test('nothing is offered where no flow asked, or the server offers no passkeys', async () => {
  accountLoads('ana')
  await nextTick()
  expect(usePasskeyOffer().open).toBe(false)

  loadPage()
  usePasskeyOffer().request()
  loadPage()
  const { user } = identitySchema.parse({
    user: { id: 'ana', email: 'ana@example.test', nickname: 'ana', role: 'master', createdAt: '2026-09-18T00:00:00.000Z', disabled: false, choosesPassword: false },
  })
  useSession().current = { status: 'signedIn', user }
  useProduct().start()
  channels.last().connect(productState({ passkeyRpId: null }))
  await nextTick()
  expect(usePasskeyOffer().open).toBe(false)
})

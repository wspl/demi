import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import type { Preferences } from '../api/generated/web-api'
import { productStateSchema } from '../api/unported'
import { usePreferences } from './preferences'
import { useProduct } from './product'

const realFetch = globalThis.fetch
const realLanguages = Object.getOwnPropertyDescriptor(navigator, 'languages')
let pinia: ReturnType<typeof createPinia>
let saved: Preferences
let patches: unknown[]

function state() {
  return productStateSchema.parse({
    user: {
      id: 'test-user',
      email: 'test@example.test',
      nickname: 'Test',
      role: 'master',
      createdAt: '2026-09-10T00:00:00.000Z',
    },
    mode: 'shared',
    preferences: saved,
    devices: [],
    workspaces: [],
    providers: [],
    conversations: [],
  })
}

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  saved = { appearance: {}, shortcuts: {} }
  patches = []
  Object.defineProperty(navigator, 'languages', {
    configurable: true,
    get: () => ['zh-cn', 'en', 'zh-CN'],
  })
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path === '/api/state') {
      return Response.json(state())
    }
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    if (path === '/api/settings/preferences' && init?.method === 'PATCH') {
      const body = JSON.parse(String(init.body))
      patches.push(body)
      saved = { ...saved, ...body }
      return Response.json({ preferences: saved })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
})

afterEach(() => {
  usePreferences().stop()
  useProduct().stop()
  disposePinia(pinia)
  globalThis.fetch = realFetch
  if (realLanguages) {
    Object.defineProperty(navigator, 'languages', realLanguages)
  } else {
    delete (navigator as { languages?: unknown }).languages
  }
})

test('the browser reports its time zone and languages once, and again when they differ from the stored ones', async () => {
  await useProduct().start()
  const preferences = usePreferences()
  const locale = {
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    languages: ['zh-CN', 'en'],
  }
  await preferences.reportLocale()
  expect(patches).toEqual([{ locale }])
  expect(useProduct().snapshot?.preferences.locale).toEqual(locale)
  await preferences.reportLocale()
  expect(patches).toHaveLength(1)

  // Another browser of the same user reported its own; this one reports again.
  saved = { ...saved, locale: { timeZone: 'Europe/Berlin', languages: ['de-DE'] } }
  await useProduct().refresh()
  await preferences.reportLocale()
  expect(patches).toEqual([{ locale }, { locale }])
})

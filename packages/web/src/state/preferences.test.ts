import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import type { Preferences } from '../api/generated/web-api'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { waitFor } from '@demicodes/utils'
import { usePreferences } from './preferences'
import { useProduct } from './product'
import { useResources } from './resources'
import { setThemeChoice } from '@demicodes/web-ui/theme/appTheme'

const realFetch = globalThis.fetch
const realLanguages = Object.getOwnPropertyDescriptor(navigator, 'languages')
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
let saved: Preferences
let patches: unknown[]
/** What `/api/models` answers. */
let catalog: unknown

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  saved = { appearance: {}, shortcuts: {} }
  patches = []
  catalog = { providers: [] }
  Object.defineProperty(navigator, 'languages', {
    configurable: true,
    get: () => ['zh-cn', 'en', 'zh-CN'],
  })
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return Response.json(catalog)
    }
    if (path === '/api/settings/preferences' && init?.method === 'PATCH') {
      const body = JSON.parse(String(init.body))
      patches.push(body)
      saved = { ...saved, ...body }
      return Response.json({ preferences: saved })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
})

afterEach(() => {
  usePreferences().stop()
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
  if (realLanguages) {
    Object.defineProperty(navigator, 'languages', realLanguages)
  } else {
    delete (navigator as { languages?: unknown }).languages
  }
})

test('the web browser reports its time zone, languages and color scheme once, and again what differs from the stored ones', async () => {
  useProduct().start()
  channels.last().connect(productState({ preferences: saved }))
  const preferences = usePreferences()
  const locale = {
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    languages: ['zh-CN', 'en'],
  }
  setThemeChoice('dark')
  await preferences.reportBrowser()
  expect(patches).toEqual([{ locale, colorScheme: 'dark' }])
  expect(useProduct().snapshot?.preferences.locale).toEqual(locale)
  await preferences.reportBrowser()
  expect(patches).toHaveLength(1)

  // Another web browser of the same user reported its own, which the channel
  // brings; this one reports again.
  saved = { ...saved, locale: { timeZone: 'Europe/Berlin', languages: ['de-DE'] } }
  channels.last().send({ type: 'preferences', preferences: saved })
  await preferences.reportBrowser()
  expect(patches).toEqual([{ locale, colorScheme: 'dark' }, { locale }])

  // The page turns light: the conversation browser starts light from now on.
  setThemeChoice('light')
  await preferences.reportBrowser()
  expect(patches.at(-1)).toEqual({ colorScheme: 'light' })
})

// A fixed bug: a backend that stored the report in another spelling than the
// page's made the page report again on every change of the stored preferences,
// requests without end.
test('a report the backend stored in its own spelling is not sent again', async () => {
  const fetchAsBefore = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path === '/api/settings/preferences' && init?.method === 'PATCH') {
      const body = JSON.parse(String(init.body))
      patches.push(body)
      // The backend's canonical form differs from the browser's.
      saved = { ...saved, ...body, ...(body.locale ? { locale: { ...body.locale, languages: ['zh-Hans-CN', 'en'] } } : {}) }
      return Response.json({ preferences: saved })
    }
    return fetchAsBefore(input, init)
  }) as typeof fetch
  useProduct().start()
  channels.last().connect(productState({ preferences: saved }))
  const preferences = usePreferences()
  setThemeChoice('dark')
  await preferences.reportBrowser()
  await preferences.reportBrowser()
  await preferences.reportBrowser()
  expect(patches).toHaveLength(1)
})

/** The provider entry `stub`, as the product state lists it. */
const stub = {
  id: 'stub', kind: 'api_key' as const, providerType: 'stub', label: 'Stub',
  wireApi: null, vendorId: null, baseUrl: null, hasApiKey: true, models: null, createdAt: '2026-09-09T00:00:00.000Z',
  details: { type: 'failed' as const, message: 'Not read in this test' },
}

/** A catalog model of `stub`. */
function catalogModel(id: string) {
  const selection = {
    providerId: 'stub', thinking: null, serviceTierId: null,
    model: { id, name: id, contextWindow: 1000, outputLimit: null, thinking: [], acceptedExtensions: [] },
  }
  return {
    id, displayName: id, description: null, contextWindow: 1000, outputLimit: null,
    supportsTools: null, acceptedExtensions: null, supportsReasoning: null, supportedThinkingEfforts: [],
    canDisableThinking: null, serviceTiers: [], defaultServiceTierId: null, cost: null, selection,
  }
}

test("a model or an entry the user turns off is saved in the user's preferences, and every page's menus leave it out", async () => {
  catalog = { providers: [{
    providerId: 'stub', displayName: 'Stub', sourceFetchedAt: '1970-01-01T00:00:00.000Z', stale: false,
    warnings: [], auth: { status: 'unknown' }, runtime: { status: 'ready' }, cliPackage: null,
    availability: { type: 'available' }, models: [catalogModel('a'), catalogModel('b')],
  }] }
  useProduct().start()
  channels.last().connect(productState({ preferences: saved, providers: [stub] }))
  const preferences = usePreferences()
  const resources = useResources()
  await waitFor(() => resources.models.stub?.length === 2, () => 'the catalog did not load')

  // Off at once in this page, and written for every other.
  preferences.showModel('stub', 'b', false)
  expect(resources.models.stub?.map((model) => model.id)).toEqual(['a'])
  expect(resources.providers[0]?.models.map((model) => model.enabled)).toEqual([true, false])
  await waitFor(() => patches.length === 1, () => 'the switch was not saved')
  expect(patches).toEqual([{ hiddenModels: [{ providerId: 'stub', modelId: 'b' }] }])

  // Another page turned the entry off; this page's menus follow.
  saved = { ...saved, hiddenProviders: ['stub'] }
  channels.last().send({ type: 'preferences', preferences: saved })
  expect(resources.providerInfos[0]?.isAvailable).toBe(false)
  expect(resources.providers[0]?.enabled).toBe(false)
})

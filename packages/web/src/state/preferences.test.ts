import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import type { Preferences } from '../api/generated/web-api'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { deferred, waitFor } from '@demicodes/utils'
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
  catalog = { providers: [], smallModel: null }
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

test('the web browser reports its time zone, languages, the language it resolves and color scheme once, and again what differs from the stored ones', async () => {
  useProduct().start()
  channels.last().connect(productState({ preferences: saved }))
  const preferences = usePreferences()
  const locale = {
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    languages: ['zh-CN', 'en'],
    // The language the page resolves for System, which the backend's mail reads.
    language: 'zh-Hans' as const,
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
  wireApi: null, vendorId: null, compatible: false, endpoint: null, hasApiKey: true, rotation: 'manual' as const, media: { image: { shape: null, enabled: true }, video: { shape: null, enabled: true }, audio: { shape: null, enabled: true }, catalog: '2026-09-01T00:00:00.000Z' }, models: null, createdAt: '2026-09-09T00:00:00.000Z',
  details: { type: 'failed' as const, message: 'Not read in this test' }, cliInstall: null,
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
    providerId: 'stub', displayName: 'Stub', sourceFetchedAt: '1970-01-01T00:00:00.000Z', imageModels: [], videoModels: [], audioModels: [], stale: false,
    warnings: [], auth: { status: 'unknown' }, runtime: { status: 'ready' }, cliPackage: null,
    availability: { type: 'available' }, models: [catalogModel('a'), catalogModel('b')],
  }], smallModel: null }
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

/** Lets the event loop turn once, which fake timers leave real. */
const turn = () => new Promise((resolve) => setImmediate(resolve))

// A change made just before the page went was lost: its write waited for the
// user's pause, which never came. The page hidden or gone flushes it at once,
// and the pause sends nothing more.
test("a slider's change waiting for the pause is sent at once when the page flushes, and once", async () => {
  jest.useFakeTimers()
  try {
    useProduct().start()
    channels.last().connect(productState({ preferences: saved }))
    const preferences = usePreferences()
    preferences.update({ appearance: { fontSize: 17 } }, { bursty: true })
    await turn()
    expect(patches).toEqual([])
    await preferences.flush()
    expect(patches).toEqual([{ appearance: { fontSize: 17 } }])
    jest.advanceTimersByTime(1_000)
    await turn()
    expect(patches).toHaveLength(1)
    expect(preferences.appearance.fontSize).toBe(17)
  } finally {
    jest.useRealTimers()
  }
})

// A fixed bug: a switch turned and the page reloaded at once lost the change,
// whose write waited for a pause and then behind the write before it. A
// switch's change is written at once, and a flush as the page goes sends
// what still waits at once, with keepalive, not behind a write in flight.
test("a switch's change is written at once, and one behind a write in flight goes at once when the page flushes", async () => {
  const held = deferred<Response>()
  const sent: { body: unknown; keepalive: boolean | undefined }[] = []
  globalThis.fetch = (async (input, init) => {
    if (String(input) === '/api/settings/preferences' && init?.method === 'PATCH') {
      const body = JSON.parse(String(init.body))
      sent.push({ body, keepalive: init.keepalive })
      saved = { ...saved, ...body }
      const answer = Response.json({ preferences: saved })
      return sent.length === 1 ? held.promise : answer
    }
    throw new Error(`Unexpected request: ${String(input)}`)
  }) as typeof fetch
  jest.useFakeTimers()
  try {
    useProduct().start()
    channels.last().connect(productState({ preferences: saved }))
    const preferences = usePreferences()

    // Ticked: written without a pause, and held on its way.
    preferences.update({ resumeAtRetry: true })
    await turn()
    expect(sent).toEqual([{ body: { resumeAtRetry: true }, keepalive: true }])

    // Unticked: it waits behind the held write until the page goes.
    preferences.update({ resumeAtRetry: false })
    await turn()
    expect(sent).toHaveLength(1)
    void preferences.flush()
    expect(sent).toEqual([
      { body: { resumeAtRetry: true }, keepalive: true },
      { body: { resumeAtRetry: false }, keepalive: true },
    ])

    // The held write ends; nothing is sent again.
    held.resolve(Response.json({ preferences: { ...saved, resumeAtRetry: true } }))
    jest.advanceTimersByTime(1_000)
    await turn()
    expect(sent).toHaveLength(2)
  } finally {
    jest.useRealTimers()
  }
})

test('Settings › Agent names Automatic’s model from the catalog, read again when what Automatic depends on changes, and a choice shows at once until Automatic clears it', async () => {
  let modelReads = 0
  const haiku = {
    providerId: 'p1',
    model: { id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5', contextWindow: 200_000, outputLimit: null, thinking: [], acceptedExtensions: null },
    thinking: null,
    serviceTierId: null,
  }
  catalog = { providers: [], smallModel: haiku }
  const fetchAsBefore = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      modelReads++
    }
    if (path === '/api/settings/preferences' && init?.method === 'PATCH') {
      // The backend stores no choice for Automatic.
      const { smallModel, ...body } = JSON.parse(String(init.body))
      patches.push({ smallModel, ...body })
      saved = { ...saved, ...body, ...(smallModel ? { smallModel } : {}) }
      if (smallModel === null) {
        delete saved.smallModel
      }
      return Response.json({ preferences: saved })
    }
    return fetchAsBefore(input, init)
  }) as typeof fetch
  const product = useProduct()
  product.start()
  channels.last().connect(productState({ preferences: saved }))
  const preferences = usePreferences()
  await waitFor(() => product.automaticSmallModel?.model.name === 'Claude Haiku 4.5')
  expect(modelReads).toBe(1)

  const chosen = { providerId: 'p1', modelId: 'claude-sonnet', thinkingEffort: 'low' }
  preferences.update({ smallModel: chosen })
  expect(preferences.smallModel).toEqual(chosen)
  preferences.update({ smallModel: null })
  expect(preferences.smallModel).toBeNull()
  await waitFor(() => patches.length === 1)
  expect(patches).toEqual([{ smallModel: null }])
  expect(product.snapshot?.preferences.smallModel).toBeUndefined()

  // A change Automatic does not depend on reads nothing; small requests
  // kept on the conversation's provider, from another page, read the
  // catalog again, which then names no model.
  channels.last().send({ type: 'preferences', preferences: { ...saved, sendWhileRunning: 'queue' } })
  catalog = { providers: [], smallModel: null }
  channels.last().send({ type: 'preferences', preferences: { ...saved, smallModelSameProvider: true } })
  await waitFor(() => product.automaticSmallModel === null)
  expect(modelReads).toBe(2)
})

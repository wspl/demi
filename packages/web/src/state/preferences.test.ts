import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import type { Preferences } from '../api/generated/web-api'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { usePreferences } from './preferences'
import { useProduct } from './product'
import { setThemeChoice } from '@demicodes/web-ui/theme/appTheme'

const realFetch = globalThis.fetch
const realLanguages = Object.getOwnPropertyDescriptor(navigator, 'languages')
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
let saved: Preferences
let patches: unknown[]

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

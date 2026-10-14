import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { useCredentialSettings } from './credentials'

const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** Every request the page sent besides the model catalog's, by its path. */
let requests: string[]

// A few milliseconds: the page's store over a played channel, no backend.
beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  requests = []
  globalThis.fetch = (async (input): Promise<Response> => {
    const path = String(input)
    if (path === '/api/models') {
      // The page reads the model catalog as it starts.
      return Response.json({ providers: [] })
    }
    requests.push(path)
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
})
afterEach(() => {
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
})

test('the list shows a visible field’s value as the channel brings it, with nothing read for it, and a secret one hidden', () => {
  useProduct().start()
  channels.last().connect(
    productState({
      credentials: [
        {
          id: 'c1',
          name: 'Linear sign-in',
          note: '',
          fields: [
            { name: 'user', label: 'User name', kind: 'visible', value: 'alex@example.com' },
            { name: 'password', label: 'Password', kind: 'secret' },
          ],
          keepUntil: null,
          lastUsedAt: null,
          createdAt: '2026-10-01T00:00:00.000Z',
          updatedAt: '2026-10-01T00:00:00.000Z',
        },
      ],
    }),
  )

  expect(useCredentialSettings().credentials[0]!.fields).toEqual([
    { name: 'user', label: 'User name', kind: 'visible', value: 'alex@example.com' },
    { name: 'password', label: 'Password', kind: 'secret', value: null },
  ])
  expect(requests).toEqual([])
})

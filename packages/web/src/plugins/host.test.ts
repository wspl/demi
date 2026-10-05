import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { z } from 'zod'
import { exposeStateSchema, type ExposeState } from '@demicodes/plugin-expose'
import { PluginCallError, definePage, pageContext } from '@demicodes/web-ui/plugins/page'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { productPageHost } from './host'

const realFetch = globalThis.fetch
const CONVERSATION = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The `expose` plugin's state the channel brings. */
let expose: ExposeState
/** Each call the backend received: its path and body. */
let calls: [string, unknown][]

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  expose = {
    available: true,
    exposes: [
      {
        id: 'k7x2maqw4p3s6tavaw2y4z6aab',
        number: 1,
        deviceId: 'laptop',
        deviceName: 'laptop',
        address: '127.0.0.1:5173',
        url: 'https://k7x2maqw4p3s6tavaw2y4z6aab.expose.demi.example/',
        expiresAt: '2026-09-17T00:59:00.000Z',
      },
    ],
  }
  calls = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    calls.push([path, JSON.parse(String(init?.body))])
    if (path === '/api/plugins/expose/calls/renew') {
      return Response.json(null)
    }
    if (path === `/api/conversations/${CONVERSATION}/plugins/browser/calls/open`) {
      return Response.json({ tab: { id: 't1', title: '', url: 'about:blank', createdBy: { kind: 'user' } } })
    }
    if (path === '/api/plugins/expose/calls/remove') {
      return Response.json(
        { code: 'plugin_refused', reason: 'expose_not_found', message: 'No expose k7x2' },
        { status: 409 },
      )
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
  const product = useProduct()
  product.start()
  channels.last().connect(productState({ pluginStates: { expose } }))
})

afterEach(() => {
  globalThis.fetch = realFetch
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
})

/** The page context of `plugin`'s page over the product's host. */
function page(plugin: string) {
  return pageContext(productPageHost(), definePage({ plugin }))
}

test("a plugin's user state is the product state's: its snapshot, each later message, and none once it is off", () => {
  const state = page('expose').plugin.state(exposeStateSchema)
  expect(state.value?.exposes.map((entry) => entry.id)).toEqual(['k7x2maqw4p3s6tavaw2y4z6aab'])
  channels.last().send({ type: 'plugin', plugin: 'expose', state: { available: true, exposes: [] } })
  expect(state.value?.exposes).toEqual([])
  const entry = productState().plugins[0]!
  channels.last().send({ type: 'plugins', plugins: [{ ...entry, enabled: false }] })
  expect(state.value).toBeNull()
})

test('a call goes to its plugin route, for the user or for a conversation, and answers its checked result', async () => {
  expect(await page('expose').plugin.call('renew', { expose: 'k7x2maqw4p3s6tavaw2y4z6aab' }, z.null())).toBeNull()
  const opened = await page('browser').plugin.conversation(CONVERSATION).call('open', {}, z.object({ tab: z.object({ id: z.string() }) }))
  expect(opened.tab.id).toBe('t1')
  expect(calls).toEqual([
    ['/api/plugins/expose/calls/renew', { expose: 'k7x2maqw4p3s6tavaw2y4z6aab' }],
    [`/api/conversations/${CONVERSATION}/plugins/browser/calls/open`, {}],
  ])
})

test("a refusal rejects with the plugin's own reason", async () => {
  const refused = page('expose').plugin.call('remove', { expose: 'k7x2maqw4p3s6tavaw2y4z6aab' }, z.null())
  await expect(refused).rejects.toBeInstanceOf(PluginCallError)
  await expect(refused).rejects.toMatchObject({ reason: 'expose_not_found', message: 'No expose k7x2' })
})

test("what a conversation holds is its primary Host's, of the plugin's packages", () => {
  const laptop = 'b5c6d7e8-0000-4000-8000-000000000001'
  const base = productState()
  channels.last().connect(productState({
    devices: [{
      id: laptop,
      kind: 'user',
      name: 'laptop',
      platform: 'linux',
      claimedAt: '2026-09-10T00:00:00.000Z',
      lastSeenAt: null,
      online: true,
      home: '/home/ada',
      installed: [
        { package: 'demi.browser', name: 'program', version: '0.1.3' },
        { package: 'demi.file', name: 'program', version: '0.1.0' },
      ],
    }],
    conversations: [
      conversationSummary(CONVERSATION, 'Work', { target: { kind: 'device', deviceId: laptop, path: '/home/ada/work' } }),
    ],
    plugins: [{ id: 'browser', name: 'Browser', description: 'A browser.', enabled: true, packages: ['demi.browser'] }],
    pluginStates: base.pluginStates,
  }))
  const browser = page('browser').plugin
  expect(browser.conversation(CONVERSATION).installed.value).toEqual([
    { package: 'demi.browser', name: 'program', version: '0.1.3' },
  ])
  // A conversation the page does not know has no Host.
  expect(browser.conversation('unknown').installed.value).toEqual([])
})

import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { z } from 'zod'
import { skillsStateSchema, type SkillsState } from '@demicodes/plugin-skills'
import { PluginCallError, definePage, pageContext } from '@demicodes/web-ui/plugins/page'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import { productPageHost } from './host'

const realFetch = globalThis.fetch
const CONVERSATION = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The `skills` plugin's state the channel brings. */
let skills: SkillsState
/** Each call the backend received: its path and body. */
let calls: [string, unknown][]

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  skills = {
    sources: [
      {
        id: 'src_1',
        origin: 'acme/tools',
        commit: 'a1b2c3d',
        fetchedAt: '2026-09-17T00:59:00.000Z',
        fetching: false,
        updateAvailable: false,
        skills: [{ name: 'review', description: 'Review a change.', warnings: [], enabled: false, disableModelInvocation: false }],
        skipped: [],
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
    if (path === '/api/plugins/skills/calls/set_enabled') {
      return Response.json(null)
    }
    if (path === `/api/conversations/${CONVERSATION}/plugins/browser/calls/open`) {
      return Response.json({ tab: { id: 't1', title: '', url: 'about:blank', createdBy: { kind: 'user' } } })
    }
    if (path === '/api/plugins/skills/calls/remove_source') {
      return Response.json(
        { code: 'plugin_refused', reason: 'source_not_found', message: 'No skill source "src_2"' },
        { status: 409 },
      )
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
  channels = playChannels()
  const product = useProduct()
  product.start()
  channels.last().connect(productState({ pluginStates: { skills } }))
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
  const state = page('skills').plugin.state(skillsStateSchema)
  expect(state.value?.sources.map((source) => source.id)).toEqual(['src_1'])
  channels.last().send({ type: 'plugin', plugin: 'skills', state: { sources: [] } })
  expect(state.value?.sources).toEqual([])
  const entry = productState().plugins[0]!
  channels.last().send({ type: 'plugins', plugins: [{ ...entry, enabled: false }] })
  expect(state.value).toBeNull()
})

test('a call goes to its plugin route, for the user or for a conversation, and answers its checked result', async () => {
  const review = { source: 'src_1', skill: 'review', enabled: true }
  expect(await page('skills').plugin.call('set_enabled', review, z.null())).toBeNull()
  const opened = await page('browser').plugin.conversation(CONVERSATION).call('open', {}, z.object({ tab: z.object({ id: z.string() }) }))
  expect(opened.tab.id).toBe('t1')
  expect(calls).toEqual([
    ['/api/plugins/skills/calls/set_enabled', review],
    [`/api/conversations/${CONVERSATION}/plugins/browser/calls/open`, {}],
  ])
})

test("a refusal rejects with the plugin's own reason", async () => {
  const refused = page('skills').plugin.call('remove_source', { source: 'src_2' }, z.null())
  await expect(refused).rejects.toBeInstanceOf(PluginCallError)
  await expect(refused).rejects.toMatchObject({ reason: 'source_not_found', message: 'No skill source "src_2"' })
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
      state: 'online',
      home: '/home/ada',
      installed: [
        { package: 'demi.browser', name: 'program', version: '0.1.3' },
        { package: 'demi.file', name: 'program', version: '0.1.0' },
      ],
      startCommand: null,
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

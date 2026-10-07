import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { z } from 'zod'
import { waitFor } from '@demicodes/utils'
import { skillsStateSchema, type SkillsState } from '@demicodes/plugin-skills'
import { PluginCallError, definePage, pageContext } from '@demicodes/web-ui/plugins/page'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useWorkPanel } from '../conversation/work'
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
/** The tabs the backend's work panel holds, as a read of it answers them. */
let panelTabs: unknown[]

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
  panelTabs = []
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return Response.json({ providers: [] })
    }
    if (path === `/api/conversations/${CONVERSATION}/panel`) {
      return Response.json({ revision: 1, tabs: panelTabs })
    }
    if (path === `/api/conversations/${CONVERSATION}/panel/changes`) {
      return Response.json({ revision: 2, changed: true })
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
      os: null,
      runnerVersion: null,
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

test("a conversation's Host is starting while it is the user's Cloud and the Cloud does not run", () => {
  const laptop = 'b5c6d7e8-0000-4000-8000-000000000001'
  const cloud = (state: 'off' | 'booting' | 'running') => ({ ...productState().cloud, device: { id: 'c1', name: 'Cloud' }, state })
  const onDevice = 'e1d2c3b4-0000-4000-8000-000000000002'
  const connect = (state: 'off' | 'booting' | 'running') => channels.last().connect(productState({
    cloud: cloud(state),
    conversations: [
      conversationSummary(CONVERSATION, 'Work'),
      conversationSummary(onDevice, 'Laptop', { target: { kind: 'device', deviceId: laptop, path: '/home/ada/work' } }),
    ],
  }))
  connect('booting')
  const skills = page('skills').plugin
  expect(skills.conversation(CONVERSATION).hostStarting.value).toBe(true)
  // A paired device never starts the way a Cloud does: offline is a Host that cannot be reached.
  expect(skills.conversation(onDevice).hostStarting.value).toBe(false)
  connect('running')
  expect(skills.conversation(CONVERSATION).hostStarting.value).toBe(false)
})

test("a link's tab stands after the tabs its opener opened before, as the backend's panel names them after a reload, and after the others once its opener is gone", async () => {
  // The panel as a reloaded page reads it: `link` was opened from `a` before the reload, by this page or another.
  panelTabs = [
    { id: 'a', kind: 'browser', data: { url: 'https://a.test/' } },
    { id: 'link', kind: 'browser', data: { url: 'https://link.test/', openedBy: 'a' } },
    { id: 'b', kind: 'browser', data: { url: 'https://b.test/' } },
  ]
  const work = useWorkPanel()
  channels.last().connect(productState({
    conversations: [conversationSummary(CONVERSATION, 'Work', { panelRevision: 1 })],
    pluginStates: { skills },
  }))
  work.load(CONVERSATION)
  const urls = () => work.stateFor(CONVERSATION).panel.tabs.map((tab) => z.object({ url: z.string() }).parse(tab.data).url)
  await waitFor(() => urls().length === 3)
  const panel = productPageHost().panel
  // Open Link in New Tab on `a` again, then once more.
  panel.add(CONVERSATION, 'browser', { url: 'https://next.test/', openedBy: 'a' }, { select: false })
  panel.add(CONVERSATION, 'browser', { url: 'https://last.test/', openedBy: 'a' }, { select: false })
  expect(urls()).toEqual(['https://a.test/', 'https://link.test/', 'https://next.test/', 'https://last.test/', 'https://b.test/'])
  panel.add(CONVERSATION, 'browser', { url: 'https://late.test/', openedBy: 'closed-meanwhile' }, { select: false })
  expect(urls().at(-1)).toBe('https://late.test/')
})

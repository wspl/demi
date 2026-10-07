import { expect, test } from 'bun:test'
import { computed } from 'vue'
import type { PreviewPlace } from '@demicodes/plugin-sdk'
import { PREVIEW_MAX_STORAGE_BYTES, type PageStorage, type PreviewOpened } from '../../generated/plugin'
import type { BrowserTabData } from '../../live/tabs'
import { PreviewConnection, type TakenState } from '../connection'
import { PreviewTabs, type PreviewDriver, type PreviewTabData } from '../tabs'

// Page states between the two browsers as the tabs of the user's browser move
// them (`preview.md` § Page state), over a scripted driver and panel: Open in
// Your Browser writes the agent's page's storage before the page boots and
// moves once; Open in Agent's Browser keeps the page's state for the agent's
// tab beside it; what does not move is a toast, and the page opens either
// way. No DOM. A few milliseconds.

const PLACE: PreviewPlace = {
  scheme: 'https',
  domain: 'demi-preview.dev',
  namespace: 'k3f9x2ab',
  host: 'host-1',
  runtime: { release: '0.1.19', url: '/runtime/0.1.19.js' },
}
const OPENED: PreviewOpened = {
  label: 'selbnt2qp6d94in3',
  environment: { origin: 'http://localhost:5173', top: 'http://localhost', cross: false },
  origin: 'https://k3f9x2ab--selbnt2qp6d94in3.demi-preview.dev',
}

function storage(skipped: string[] = []): PageStorage {
  return { origin: 'http://localhost:5173', local: [{ key: 'session', value: 'signed-in' }], session: [], databases: [], skipped }
}

/** A conversation's previews whose driver does as `script` says, recording what it was asked. */
function world(script: Partial<PreviewDriver> = {}) {
  const steps: string[] = []
  const notices: [string, string][] = []
  const added: PreviewTabData[] = []
  const agentTabs: BrowserTabData[] = []
  const driver: PreviewDriver = {
    boots: true,
    register: () => () => {},
    async boot(_tab, _place, opened, navigation) {
      steps.push(`boot ${navigation.url}`)
      return `${opened.origin}/__demi/v1/boot.html`
    },
    command: () => false,
    icon: async () => null,
    async takeState(_tab, _place, from) {
      steps.push(`take ${from}`)
      return { url: 'http://localhost:5173/app', title: 'App', mobile: false, storage: storage(), tooLarge: false } satisfies TakenState
    },
    async writeState(_tab, opened, written) {
      steps.push(`write ${opened.origin} ${written.local[0]?.value}`)
      return []
    },
    readState: async () => storage(),
    origins: () => ['http://localhost:5173', 'http://api.localhost:5173'],
    async keepState(_tab, _place, token, origins, kept) {
      steps.push(`keep ${token === agentTabs.at(-1)?.handover ? 'the tab’s token' : token} ${origins.join(' ')} ${kept === null ? 'cookies only' : 'with storage'}`)
    },
    ...script,
  }
  const tabs = new PreviewTabs(
    {
      place: computed(() => PLACE),
      connection: new PreviewConnection(() => ({ send() {}, close() {} })),
      hostStarting: () => false,
      open: async () => OPENED,
      add: (data) => added.push(data),
      addAgentTab: (data) => agentTabs.push(data),
      notify: (title, message) => notices.push([title, message]),
    },
    driver,
    () => null,
  )
  /** A tab's content starting with `data`, whose data the tab updates. */
  function attach(id: string, initial: PreviewTabData) {
    let data = initial
    const tab = tabs.attach(id, {
      frame: () => null,
      data: () => data,
      update: (next) => {
        data = next
      },
      close: () => {},
    })
    return { tab, data: () => data }
  }
  return { tabs, steps, notices, added, agentTabs, attach }
}

async function settled(): Promise<void> {
  for (let turn = 0; turn < 10; turn++) {
    await Promise.resolve()
  }
}

test('Open in Your Browser writes the agent’s page state before the page boots, and only once', async () => {
  const { tabs, steps, added, attach } = world()
  tabs.fromAgent('browser-t3', { url: 'http://localhost:5173/app', tab: 't3', title: 'App' })
  expect(added).toEqual([{ url: 'http://localhost:5173/app', title: 'App', openedBy: 'browser-t3', from: 't3' }])
  const { data } = attach('p1', added[0]!)
  await settled()
  expect(steps).toEqual(['take t3', `write ${OPENED.origin} signed-in`, 'boot http://localhost:5173/app'])
  // A reload opens the address alone: the state moved once.
  expect(data().from).toBeUndefined()
  // A tab the agent's browser has none of yet offers nothing to take.
  tabs.fromAgent('browser-t4', { url: 'about:blank' })
  expect(added).toHaveLength(1)
})

test('what of the page’s state did not move is a toast, and the page opens either way', async () => {
  const partial = world({
    takeState: async () => ({ url: 'http://localhost:5173/', title: 'App', mobile: false, storage: storage(['keys/crypto']), tooLarge: false }),
    writeState: async () => ['drafts'],
  })
  partial.attach('p1', { url: 'http://localhost:5173/', from: 't3' })
  await settled()
  expect(partial.notices).toEqual([
    ['Some of the Page’s Storage Didn’t Move', 'On localhost:5173, keys/crypto has a value that can’t be copied; drafts is open in another tab.'],
  ])

  const large = world({ takeState: async () => ({ url: 'http://localhost:5173/', title: 'App', mobile: false, storage: null, tooLarge: true }) })
  large.attach('p1', { url: 'http://localhost:5173/', from: 't3' })
  await settled()
  expect(large.notices).toEqual([['Only the Page’s Cookies Moved', 'Its storage is larger than the 16 MB a page state moves.']])
  expect(large.steps).toEqual(['boot http://localhost:5173/'])

  const gone = world({ takeState: () => Promise.reject(new Error('The agent’s tab is gone.')) })
  gone.attach('p1', { url: 'http://localhost:5173/', from: 't3' })
  await settled()
  expect(gone.notices).toEqual([['The Page Opened Without Its State', 'The agent’s tab is gone.']])
  expect(gone.steps).toEqual(['boot http://localhost:5173/'])
})

test('Open in Agent’s Browser opens the agent’s tab at once, and keeps the page’s state for it meanwhile', async () => {
  let reading: () => void = () => {}
  const read = new Promise<void>((resolve) => {
    reading = resolve
  })
  const { tabs, steps, agentTabs, attach } = world({
    readState: async () => {
      await read
      return storage()
    },
  })
  const { tab } = attach('p1', { url: 'http://localhost:5173/app' })
  await settled()
  tab.report({ type: 'page', page: { url: 'http://localhost:5173/app#done', title: 'App', icon: '' } })
  const handing = tabs.toAgent('p1')
  // The tab is there before the page's storage is read.
  expect(agentTabs).toEqual([{ url: 'http://localhost:5173/app#done', openedBy: 'p1', handover: expect.any(String) }])
  reading()
  await handing
  expect(steps.at(-1)).toBe('keep the tab’s token http://localhost:5173 http://api.localhost:5173 with storage')
})

test('Open in Agent’s Browser moves cookies only past the size a page state moves, and the address alone when its state fails', async () => {
  const huge = { ...storage(), local: [{ key: 'blob', value: 'x'.repeat(PREVIEW_MAX_STORAGE_BYTES) }] }
  const large = world({ readState: async () => huge })
  const shown = large.attach('p1', { url: 'http://localhost:5173/' })
  await settled()
  shown.tab.report({ type: 'page', page: { url: 'http://localhost:5173/', title: 'App', icon: '' } })
  await large.tabs.toAgent('p1')
  expect(large.steps.at(-1)).toBe('keep the tab’s token http://localhost:5173 http://api.localhost:5173 cookies only')
  expect(large.notices).toEqual([['Only the Page’s Cookies Moved', 'Its storage is larger than the 16 MB a page state moves.']])

  const failing = world({ keepState: () => Promise.reject(new Error('the preview stream ended')) })
  const page = failing.attach('p1', { url: 'http://localhost:5173/' })
  await settled()
  page.tab.report({ type: 'page', page: { url: 'http://localhost:5173/', title: 'App', icon: '' } })
  await failing.tabs.toAgent('p1')
  expect(failing.agentTabs).toEqual([{ url: 'http://localhost:5173/', openedBy: 'p1', handover: expect.any(String) }])
  expect(failing.notices).toEqual([['The Page Opened Without Its State', 'The preview stream ended.']])

  // A page whose storage cannot be read moves nothing: the tab opens its address alone.
  const unread = world({ readState: () => Promise.reject(new Error('The preview took too long to read or write the page’s storage.')) })
  const shownPage = unread.attach('p1', { url: 'http://localhost:5173/' })
  await settled()
  shownPage.tab.report({ type: 'page', page: { url: 'http://localhost:5173/', title: 'App', icon: '' } })
  await unread.tabs.toAgent('p1')
  expect(unread.steps.at(-1)).toBe('keep the tab’s token  cookies only')
  expect(unread.notices).toEqual([['The Page Opened Without Its State', 'The preview took too long to read or write the page’s storage.']])
})

test('the size mode moves with the page both ways: Mobile stays Mobile', async () => {
  const fromAgent = world({ takeState: async () => ({ url: 'http://localhost:5173/', title: 'App', mobile: true, storage: null, tooLarge: false }) })
  const { data } = fromAgent.attach('p1', { url: 'http://localhost:5173/', from: 't3' })
  await settled()
  // The page boots as a phone's, from its first request.
  expect(data()).toEqual({ url: 'http://localhost:5173/', mobile: true })
  expect(fromAgent.steps).toEqual(['boot http://localhost:5173/'])

  const toAgent = world()
  const shown = toAgent.attach('p1', { url: 'http://localhost:5173/', mobile: true })
  await settled()
  shown.tab.report({ type: 'page', page: { url: 'http://localhost:5173/', title: 'App', icon: '' } })
  await toAgent.tabs.toAgent('p1')
  expect(toAgent.agentTabs).toEqual([{ url: 'http://localhost:5173/', openedBy: 'p1', handover: expect.any(String), mobile: true }])
})

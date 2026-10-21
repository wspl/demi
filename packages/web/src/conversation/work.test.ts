import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import type { PanelTab } from '@demicodes/web-ui/agent/panel-tabs'

// The work store on a page of happy-dom, whose width the tests turn between
// the desktop's and a phone's (`phone.ts`), and whose address and history a
// router over the page's own history keeps; the panel's routes are stubbed.
// A few ms each, besides the start of the page.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/chat' })

/** The phone switch: `turn` changes it and tells the page, as a resize past 768 px does. */
class Width extends EventTarget {
  matches = false
  turn(phone: boolean): void {
    this.matches = phone
    this.dispatchEvent(Object.assign(new Event('change'), { matches: phone }))
  }
}
const width = new Width()
Object.defineProperty(window, 'matchMedia', { value: () => width, configurable: true })

const { createPinia, disposePinia, setActivePinia } = await import('pinia')
const { defineComponent, nextTick } = await import('vue')
const { z } = await import('zod')
const { waitFor } = await import('@demicodes/utils')
const { createRouter, createWebHistory } = await import('vue-router')
const { useSession } = await import('../auth/session')
const { identitySchema, panelReadSchema } = await import('../api/generated/web-api')
const { readLocalState } = await import('../state/local')
const { useResources } = await import('../state/resources')
const { useProduct } = await import('../state/product')
const { conversationSummary, productState } = await import('../__tests__/product-state')
const { showMetadata } = await import('../__tests__/shown-metadata')
const { useConversations } = await import('./store')
const { useWorkPanel } = await import('./work')

const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
const saved = new Map<string, string>()
let pinia: ReturnType<typeof createPinia>

beforeEach(() => {
  width.matches = false
  saved.clear()
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => saved.get(key) ?? null,
      setItem: (key: string, value: string) => saved.set(key, value),
    },
  })
  pinia = createPinia()
  setActivePinia(pinia)
})

afterEach(() => {
  disposePinia(pinia)
  if (storageDescriptor) {
    Object.defineProperty(globalThis, 'localStorage', storageDescriptor)
  } else {
    Reflect.deleteProperty(globalThis, 'localStorage')
  }
})

function signIn(id: string): void {
  const { user } = identitySchema.parse({ user: {
    id,
    email: `${id}@example.test`,
    nickname: id,
    role: 'master',
    createdAt: '2026-09-18T00:00:00.000Z',
    disabled: false,
    choosesPassword: false,
  } })
  useSession().current = { status: 'signedIn', user }
}

test("the dock windows' share of the session's height survives reload, for the account that set it", async () => {
  signIn('one')
  const resources = useResources()
  expect(resources.dockWindowShare).toBe(0.5)
  resources.dockWindowShare = 0.8
  await nextTick()
  expect(readLocalState('one').dockWindowShare).toBe(0.8)

  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  signIn('one')
  expect(useResources().dockWindowShare).toBe(0.8)

  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  signIn('two')
  expect(useResources().dockWindowShare).toBe(0.5)
})

test('panel open and closed choices survive reload with the panel share in the same preferences', async () => {
  signIn('one')
  const work = useWorkPanel()
  expect(work.stateFor('a').open).toBe(false)
  useResources().asideShare = 0.5
  work.setOpen('a', true)
  await nextTick()
  expect(readLocalState('one')).toMatchObject({ asideShare: 0.5, workPanelOpen: { a: true } })

  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  signIn('one')
  const restored = useWorkPanel()
  expect(restored.stateFor('a').open).toBe(true)
  expect(restored.stateFor('b').open).toBe(false)
  expect(useResources().asideShare).toBe(0.5)
  restored.setOpen('a', false)
  await nextTick()
  expect(readLocalState('one').workPanelOpen?.a).toBe(false)
})

test('the changes of a request open in the Change view with the panel, only while the changes plugin is on', async () => {
  signIn('one')
  const product = useProduct()
  const changes = { id: 'changes', name: 'Changes', description: 'Shows changes.', enabled: false, packages: [] }
  product.read = productState({ plugins: [changes] })
  const work = useWorkPanel()
  const state = work.stateFor('a')
  const edit = { node: null, request: 'user', file: 'index.ts', edit: { call: 'call', path: 'index.ts', segment: 0 } }
  // Off, nothing opens the edit: the transcript's pills are no controls.
  expect(work.canOpen('edit')).toBe(false)
  work.openIn('a', { intent: 'edit', payload: edit })
  expect(state.open).toBe(false)

  product.read = productState({ plugins: [{ ...changes, enabled: true }] })
  expect(work.canOpen('edit')).toBe(true)
  work.openIn('a', { intent: 'edit', payload: edit })
  await nextTick()
  expect(readLocalState('one').workPanelOpen?.a).toBe(true)
  // The edit takes the selection in the Change view's pinned tab; nothing is saved over a panel that was never read.
  expect(state.panel.history).toEqual(['change'])
  expect(state.pinned.change).toMatchObject({ mode: 'conversation', request: { request: 'user', file: 'index.ts' } })
  signIn('two')
  await nextTick()
  expect(state.open).toBe(false)
  signIn('one')
  await nextTick()
  expect(state.open).toBe(true)
})

test('a pill or a file link opens what it names, and on what the panel already shows leaves it as it is', () => {
  signIn('one')
  const plugin = (id: string) => ({ id, name: id, description: 'A plugin.', enabled: true, packages: [] })
  useProduct().read = productState({ plugins: [plugin('changes'), plugin('file-browser')] })
  const work = useWorkPanel()
  const state = work.stateFor('a')
  const edit = (segment: number) => ({ node: null, request: 'user', file: 'login.ts', edit: { call: 'call', path: 'login.ts', segment } })
  const first = { intent: 'edit', payload: edit(0) } as const

  // The login.ts pill shows its first edit; a second click leaves the panel
  // showing it, its data the same object, so the Change view does not redraw.
  work.openIn('a', first)
  expect(state.open).toBe(true)
  const shown = state.pinned.change
  work.openIn('a', first)
  expect(state.open).toBe(true)
  expect(state.panel.history).toEqual(['change'])
  expect(state.pinned.change).toBe(shown)
  // The user stepped to another edit: the pill shows its edit again.
  work.openIn('a', { intent: 'edit', payload: edit(1) })
  work.openIn('a', first)
  expect(state.pinned.change).toMatchObject({ mode: 'conversation', request: { edit: { segment: 0 } } })
  // Closed by its own control, the panel opens again on the pill as it was.
  const again = state.pinned.change
  work.setOpen('a', false)
  work.openIn('a', first)
  expect(state.open).toBe(true)
  expect(state.pinned.change).toBe(again)

  // A file link: another file shows instead, and the file shown again stays.
  work.openIn('a', { intent: 'file', payload: { path: '/work/a.md' } })
  work.openIn('a', { intent: 'file', payload: { path: '/work/b.md' } })
  expect(state.panel.history.at(-1)).toBe('file')
  expect(state.pinned.file).toMatchObject({ path: '/work/b.md' })
  const file = state.pinned.file
  work.openIn('a', { intent: 'file', payload: { path: '/work/b.md' } })
  expect(state.open).toBe(true)
  expect(state.pinned.file).toBe(file)
  // A link to a line: another line of the file shown moves to it, the same line stays.
  work.openIn('a', { intent: 'file', payload: { path: '/work/b.md', line: 7 } })
  expect(state.pinned.file).toMatchObject({ path: '/work/b.md', line: 7 })
  const line = state.pinned.file
  work.openIn('a', { intent: 'file', payload: { path: '/work/b.md', line: 7 } })
  expect(state.open).toBe(true)
  expect(state.pinned.file).toBe(line)
  // Shown in the Change view while the File view is selected, the edit is selected.
  work.openIn('a', first)
  expect(state.open).toBe(true)
  expect(state.panel.history.at(-1)).toBe('change')
})

test('a link’s Open in Work Panel opens its page in a new tab of the conversation’s browser with the panel shown, and Show in Files asks the File view for its tree', () => {
  signIn('one')
  const plugin = (id: string) => ({ id, name: id, description: 'A plugin.', enabled: true, packages: [] })
  useProduct().read = productState({ plugins: [plugin('browser'), plugin('file-browser')] })
  const work = useWorkPanel()
  const state = work.stateFor('a')
  expect(work.canOpen('web')).toBe(true)

  work.openIn('a', { intent: 'web', payload: { url: 'https://example.com/docs' } })
  expect(state.open).toBe(true)
  const opened = state.panel.tabs.filter((tab) => tab.kind === 'browser')
  expect(opened.map((tab) => tab.data)).toEqual([{ url: 'https://example.com/docs' }])
  expect(state.panel.history.at(-1)).toBe(opened[0]!.id)
  // Another page is another tab, as a browser opens each link in a tab of its own.
  work.openIn('a', { intent: 'web', payload: { url: 'https://example.com/api' } })
  expect(state.panel.tabs.filter((tab) => tab.kind === 'browser').map((tab) => tab.data)).toEqual([
    { url: 'https://example.com/docs' },
    { url: 'https://example.com/api' },
  ])

  work.setOpen('a', false)
  work.openIn('a', { intent: 'file', payload: { path: '/work/src/a.ts', tree: true } })
  expect(state.open).toBe(true)
  expect(state.panel.history.at(-1)).toBe('file')
  expect(state.pinned.file).toMatchObject({ path: '/work/src/a.ts', tree: true })
})

// An attachment block's card opens its file in the work panel: a tab of its own named by its file,
// which a later click on the card selects again, whatever the panel shows meanwhile, instead of
// adding another (`packages/web-ui/src/plugins/intents.ts`). Planted defect this catches: a kind
// without a key, which adds a second tab. A few ms.
test('an attachment’s card opens it in a tab of its own, and a later click selects that tab again', () => {
  signIn('one')
  const plugin = (id: string) => ({ id, name: id, description: 'A plugin.', enabled: true, packages: [] })
  useProduct().read = productState({ plugins: [plugin('file-browser')] })
  const work = useWorkPanel()
  const state = work.stateFor('a')
  expect(work.canOpen('attachment')).toBe(true)
  const report = { id: 'a3', name: 'report.pdf', size: 2516582, url: '/api/blobs/3?type=application%2Fpdf' }
  const attachmentTabs = () => state.panel.tabs.filter((tab) => tab.kind === 'attachment')

  work.openIn('a', { intent: 'attachment', payload: report })
  expect(state.open).toBe(true)
  expect(attachmentTabs().map((tab) => tab.data)).toEqual([report])
  const first = attachmentTabs()[0]!.id
  expect(state.panel.history.at(-1)).toBe(first)
  // Another attachment is another tab, which the panel then shows.
  work.openIn('a', { intent: 'attachment', payload: { ...report, id: 'a4', name: 'notes.md', size: 496 } })
  expect(attachmentTabs().map((tab) => tab.data)).toMatchObject([{ name: 'report.pdf' }, { name: 'notes.md' }])
  expect(state.panel.history.at(-1)).not.toBe(first)
  // The first card again: its tab, not a third.
  work.openIn('a', { intent: 'attachment', payload: report })
  expect(attachmentTabs()).toHaveLength(2)
  expect(state.panel.history.at(-1)).toBe(first)
})

/**
 * Answers the work panel's change route as the backend does, from `stored`,
 * keeping each change it was sent; a read of the panel, which its
 * conversation's metadata brings, is counted and refused.
 */
function stubPanelRoutes(stored: { revision: number; tabs: PanelTab[] }) {
  const route = { stored, reads: 0, sent: [] as { method: string; path: string; body: unknown }[] }
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = new URL(String(input), 'http://page.test').pathname
    const method = init?.method ?? 'GET'
    if (method === 'GET') {
      route.reads += 1
      return Response.json({ code: 'not_found', message: 'No such route' }, { status: 404 })
    }
    route.sent.push({ method, path, body: init?.body ? JSON.parse(String(init.body)) : null })
    route.stored = { ...route.stored, revision: route.stored.revision + 1 }
    return Response.json({ revision: route.stored.revision, changed: true })
  }) as typeof fetch
  return route
}

const fetchDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'fetch')

afterEach(() => {
  if (fetchDescriptor) {
    Object.defineProperty(globalThis, 'fetch', fetchDescriptor)
  }
})

async function settled(): Promise<void> {
  for (let turn = 0; turn < 20; turn++) {
    await Promise.resolve()
  }
}

test('a new conversation\'s panel reads and sends nothing until its first send creates the record', async () => {
  signIn('one')
  const route = stubPanelRoutes({ revision: 0, tabs: [] })
  const conversations = useConversations()
  const id = conversations.create()
  const work = useWorkPanel()
  work.load(id)
  const tab = work.add(id, 'page', { url: 'https://example.test/' })
  await settled()
  // The tab shows at once, selected, and nothing reached the backend.
  expect(work.stateFor(id).panel).toEqual({ history: [tab], tabs: [{ id: tab, kind: 'page', data: { url: 'https://example.test/' } }] })
  expect(work.recorded(id)).toBe(false)
  expect(route.reads).toBe(0)
  expect(route.sent).toEqual([])

  // What the first send's record does: what the user opened before is sent.
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  work.load(id)
  await settled()
  expect(route.reads).toBe(0)
  expect(route.sent).toEqual([{
    method: 'POST',
    path: `/api/conversations/${id}/panel/changes`,
    body: { changes: [{ op: 'create', id: tab, kind: 'page', data: { url: 'https://example.test/' } }] },
  }])
})

test('the selection is this page\'s own, and the metadata brings another page\'s tab with no read', async () => {
  signIn('one')
  const route = stubPanelRoutes({ revision: 1, tabs: [{ id: 'p1', kind: 'page', data: { url: 'https://a.test/' } }] })
  const conversations = useConversations()
  const id = conversations.create()
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  const product = useProduct()
  const summary = (panelRevision: number) => {
    useProduct().read = productState({ conversations: [conversationSummary(id, '')] })
    showMetadata(id, { panel: panelReadSchema.parse({ states: [], ...route.stored, revision: panelRevision }) })
  }
  summary(1)
  const work = useWorkPanel()
  work.load(id)
  await settled()
  expect(work.stateFor(id).panel.tabs.map((tab) => tab.id)).toEqual(['p1'])
  work.select(id, 'p1')
  await nextTick()
  expect(readLocalState('one').workPanelHistory).toEqual({ [id]: ['p1'] })

  // Another page added a tab: the metadata brings it.
  route.stored = { revision: 2, tabs: [...route.stored.tabs, { id: 'p2', kind: 'page', data: { url: 'https://b.test/' } }] }
  summary(2)
  await settled()
  expect(work.stateFor(id).panel).toEqual({
    history: ['p1'],
    tabs: [
      { id: 'p1', kind: 'page', data: { url: 'https://a.test/' } },
      { id: 'p2', kind: 'page', data: { url: 'https://b.test/' } },
    ],
  })

  // Closing the tab the page went to last leaves the one before it, which a reload shows again.
  expect(route.reads).toBe(0)
  work.select(id, 'p2')
  work.closeTabs(id, ['p2'])
  await nextTick()
  expect(readLocalState('one').workPanelHistory).toEqual({ [id]: ['p1'] })
})

test('a tab its kind asks to show opens the closed panel and is selected once, and a reload does not apply it again', async () => {
  signIn('one')
  const tabs = (shows?: number): PanelTab[] => [
    { id: 'b1', kind: 'browser', data: { url: 'https://a.test/', tab: 't1', ...(shows ? { shows } : {}) } },
    { id: 'b2', kind: 'browser', data: { url: 'https://b.test/', tab: 't2' } },
  ]
  let route = stubPanelRoutes({ revision: 1, tabs: tabs() })
  const conversations = useConversations()
  const id = conversations.create()
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  const product = useProduct()
  const summary = (panelRevision: number) => {
    useProduct().read = productState({
      conversations: [conversationSummary(id, '')],
      plugins: [{ id: 'browser', name: 'Browser', description: 'A browser.', enabled: true, packages: [] }],
    })
    showMetadata(id, { panel: panelReadSchema.parse({ states: [], ...route.stored, revision: panelRevision }) })
  }
  summary(1)
  const work = useWorkPanel()
  work.load(id)
  await settled()
  work.select(id, 'b2')
  expect(work.stateFor(id).open).toBe(false)

  // The agent shows t1 while the panel is closed: the plugin raises the tab's count.
  route.stored = { revision: 2, tabs: tabs(1) }
  summary(2)
  await settled()
  expect([work.stateFor(id).open, work.stateFor(id).history.at(-1)]).toEqual([true, 'b1'])
  // The user's own selection after it stands, however often the panel is read again.
  work.select(id, 'b2')
  route.stored = { revision: 3, tabs: tabs(1) }
  summary(3)
  await settled()
  expect(work.stateFor(id).history.at(-1)).toBe('b2')
  expect(readLocalState('one').workPanelShown).toEqual({ [id]: { b1: 1 } })

  // A reloaded page knows what it applied.
  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  signIn('one')
  route = stubPanelRoutes({ revision: 3, tabs: tabs(1) })
  useConversations().items.push(...conversations.items)
  summary(3)
  const reloaded = useWorkPanel()
  reloaded.setOpen(id, false)
  reloaded.load(id)
  await settled()
  expect([reloaded.stateFor(id).open, reloaded.stateFor(id).history.at(-1)]).toEqual([false, 'b2'])
  // A new showing shows it again.
  route.stored = { revision: 4, tabs: tabs(2) }
  summary(4)
  await settled()
  expect([reloaded.stateFor(id).open, reloaded.stateFor(id).history.at(-1)]).toEqual([true, 'b1'])
})

// Production: on a phone, where the panel covers the conversation, opening a
// conversation from the sidebar sometimes brought its panel up full screen by
// itself: the phone read the open state the desktop saved for it, and a tab
// the agent showed while the user was in another conversation opened it once
// the page read that conversation's panel.
test('on a phone a conversation comes up without its panel; only a show while the user is on it opens it, and the desktop keeps its choice', async () => {
  signIn('one')
  const conversations = useConversations()
  // The page keeps one new conversation: each is saved before the next.
  const [a, b] = [0, 1].map(() => {
    const id = conversations.create()
    conversations.items.find((item) => item.id === id)!.persistence = 'synced'
    return id
  }) as [string, string]
  // The desktop left A's panel open.
  useResources().local.workPanelOpen = { [a]: true }
  const browserTab = (shows: number): PanelTab[] => [
    { id: 'b1', kind: 'browser', data: { url: 'https://a.test/', tab: 't1', shows } },
    { id: 'b2', kind: 'browser', data: { url: 'https://b.test/', tab: 't2' } },
  ]
  const stored: Record<string, { revision: number; tabs: PanelTab[] }> = {
    [a]: { revision: 1, tabs: [] },
    [b]: { revision: 1, tabs: browserTab(1) },
  }
  const route = stubPanelRoutes({ revision: 0, tabs: [] })
  const product = useProduct()
  const summaries = () => {
    product.read = productState({
      conversations: [a, b].map((id) => conversationSummary(id, '')),
      plugins: [{ id: 'browser', name: 'Browser', description: 'A browser.', enabled: true, packages: [] }],
    })
    for (const id of [a, b]) {
      showMetadata(id, { panel: panelReadSchema.parse({ states: [], ...stored[id]! }) })
    }
  }
  summaries()
  const work = useWorkPanel()
  const router = attachRouter(work)
  width.turn(true)
  // As the frame does once the address shows the conversation.
  const open = async (id: string) => {
    await arrive(router, `/chat/${id}`, () => {
      work.shown(id)
      work.load(id)
    })
    return settled()
  }

  await open(a)
  expect(work.stateFor(a).open).toBe(false)
  // The agent showed b1 in B before the user came to it: it waits in the strip, selected.
  await open(b)
  expect([work.stateFor(b).open, work.stateFor(b).history.at(-1)]).toEqual([false, 'b1'])
  // Shown again while the user is on B, it opens.
  work.select(b, 'b2')
  stored[b] = { revision: 2, tabs: browserTab(2) }
  summaries()
  await settled()
  expect([work.stateFor(b).open, work.stateFor(b).history.at(-1)]).toEqual([true, 'b1'])
  // Left with its panel open, B comes back without it.
  await open(a)
  await open(b)
  expect(work.stateFor(b).open).toBe(false)
  // Shown again while the user looks at A's panel, A's stays and B comes up closed.
  await open(a)
  work.setOpen(a, true)
  stored[b] = { revision: 3, tabs: browserTab(3) }
  summaries()
  await settled()
  expect(work.stateFor(a).open).toBe(true)
  await open(b)
  expect(work.stateFor(b).open).toBe(false)

  // At the desktop's width A's panel is open as the desktop left it, and nothing the phone did was saved.
  width.turn(false)
  expect(work.stateFor(a).open).toBe(true)
  await settled()
  expect(readLocalState('one').workPanelOpen).toEqual({ [a]: true })
  expect(route.reads).toBe(0)
})

const Page = defineComponent({ render: () => null })
/** What a test's routers leave behind: each lets go of the page's history and of the store. */
const detached: (() => void)[] = []

afterEach(() => {
  for (const detach of detached.splice(0)) {
    detach()
  }
})

/** A router over the page's own history, as the product's, which holds `work`'s panel. */
function attachRouter(work: ReturnType<typeof useWorkPanel>) {
  const router = createRouter({ history: createWebHistory(), routes: [{ path: '/chat/:id?', component: Page }] })
  const stop = work.attach(router)
  detached.push(() => {
    stop()
    router.options.history.destroy()
  })
  return router
}

/** The end of the router's next navigation, the store's own or the history's (Back, Forward). */
function navigated(router: ReturnType<typeof createRouter>): Promise<void> {
  return new Promise((resolve) => {
    const stop = router.afterEach(() => {
      stop()
      resolve()
    })
  })
}

/** The page's address now, as its user reads it. */
const address = () => decodeURIComponent(window.location.pathname + window.location.search)

/** A page signed in with Files and Changes on, its store at `/chat/<id>`. */
async function panelPage(id: string) {
  signIn('one')
  const plugin = (name: string) => ({ id: name, name, description: 'A plugin.', enabled: true, packages: [] })
  useProduct().read = productState({ plugins: [plugin('changes'), plugin('file-browser')] })
  const work = useWorkPanel()
  const router = attachRouter(work)
  await arrive(router, `/chat/${id}`)
  return { work, router }
}

/**
 * Opens `path` as a link does, does what the frame does once the address
 * shows it (`shown`), and waits until the panel has said in the address, in
 * its place, what it shows there.
 */
async function arrive(router: ReturnType<typeof createRouter>, path: string, shown = () => {}): Promise<void> {
  const arrived = navigated(router)
  void router.push(path)
  await arrived
  shown()
  await waitFor(() => z.object({ workPanel: z.literal(true) }).safeParse(window.history.state).success)
}

/** Does `act`, a move of the store or of the history, and waits for the router to reach where it goes. */
async function moving(router: ReturnType<typeof createRouter>, act: () => void): Promise<void> {
  const arrived = navigated(router)
  act()
  await arrived
}

// The panel's place is in the page's address, so a reload shows the panel
// where it was (`docs/decisions/0014-the-work-panel-has-addresses.md`).
// Planted defect this catches: an address that names the tab and not its
// place, which a reload shows at the File view's start.
test('a reload shows the panel at the file it showed', async () => {
  const { work, router } = await panelPage('a')
  await moving(router, () => work.openIn('a', { intent: 'file', payload: { path: '/work/src/a.ts', line: 3 } }))
  expect(address()).toBe('/chat/a?panel=file&path=/work/src/a.ts&line=3&view=preview')

  // The reload: a new page at the same address, its memory empty.
  disposePinia(pinia)
  pinia = createPinia()
  setActivePinia(pinia)
  const reloaded = await panelPage('a?panel=file&path=/work/src/a.ts&line=3&view=preview')
  const state = reloaded.work.stateFor('a')
  expect([state.open, state.panel.history.at(-1)]).toEqual([true, 'file'])
  expect(state.pinned.file).toEqual({ path: '/work/src/a.ts', line: 3, view: 'preview' })
})

// Planted defect this catches: a file opened in the File view in place of
// the address, as a change that is no step, which leaves Back nothing to
// return to in the panel.
test('Back after another file opened in the File view returns to the first, and Forward to the second', async () => {
  const { work, router } = await panelPage('a')
  await moving(router, () => work.openIn('a', { intent: 'file', payload: { path: '/work/a.md' } }))
  // The view's own pick, as its tree emits it.
  await moving(router, () => work.updatePinned('a', 'file', { path: '/work/b.md' }))
  expect(address()).toBe('/chat/a?panel=file&path=/work/b.md&view=preview')
  expect(work.canGo('a', 'back')).toBe(true)

  await moving(router, () => work.go('back'))
  expect(address()).toBe('/chat/a?panel=file&path=/work/a.md&view=preview')
  expect(work.stateFor('a').pinned.file).toEqual({ path: '/work/a.md', view: 'preview' })
  expect(work.canGo('a', 'forward')).toBe(true)
  await moving(router, () => work.go('forward'))
  expect(work.stateFor('a').pinned.file).toEqual({ path: '/work/b.md', view: 'preview' })
})

// Planted defect this catches: Back offered whenever the history has an
// entry before, which from the panel's first step closes the panel, or
// leaves the conversation, from inside its own address bar.
test('the address bar’s Back is unavailable at the conversation’s first panel step', async () => {
  const { work, router } = await panelPage('a')
  await moving(router, () => work.setOpen('a', true))
  expect(address()).toBe('/chat/a?panel=change&view=diff')
  // The entry before is the conversation with its panel closed.
  expect(work.canGo('a', 'back')).toBe(false)
  await moving(router, () => work.openIn('a', { intent: 'file', payload: { path: '/work/a.md' } }))
  expect(work.canGo('a', 'back')).toBe(true)
  expect(work.canGo('a', 'forward')).toBe(false)

  // Another conversation's panel is not this one's: its first step offers no Back either.
  await moving(router, () => void router.push('/chat/b'))
  await moving(router, () => work.setOpen('b', true))
  expect(work.canGo('b', 'back')).toBe(false)
})

// Planted defect this catches: a selection the address names kept after the
// page read that the tab is gone, which leaves the address naming a tab the
// panel cannot show.
test('a link naming a closed tab shows what the panel would without it, its address corrected in place', async () => {
  signIn('one')
  const route = stubPanelRoutes({ revision: 1, tabs: [{ id: 'p1', kind: 'page', data: { url: 'https://example.test/' } }] })
  const conversations = useConversations()
  const id = conversations.create()
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  const plugin = (name: string) => ({ id: name, name, description: 'A plugin.', enabled: true, packages: [] })
  useProduct().read = productState({
    conversations: [conversationSummary(id, '')],
    plugins: [plugin('changes'), plugin('file-browser')],
  })
  showMetadata(id, { panel: panelReadSchema.parse({ states: [], ...route.stored }) })
  const work = useWorkPanel()
  const router = attachRouter(work)
  await moving(router, () => void router.push(`/chat/${id}?panel=t7`))
  const length = window.history.length
  // Until the page reads the tabs, the address names what may be there.
  expect(address()).toBe(`/chat/${id}?panel=t7`)

  const corrected = navigated(router)
  work.load(id)
  await corrected
  expect(address()).toBe(`/chat/${id}?panel=change&view=diff`)
  expect(window.history.length).toBe(length)
  expect(work.stateFor(id).open).toBe(true)
})

// Planted defect this catches: every update of a tab a step, so Preview and
// Source each add one that Back must step through.
test('Preview or Source changes the address in place, with no step', async () => {
  const { work, router } = await panelPage('a')
  await moving(router, () => work.openIn('a', { intent: 'file', payload: { path: '/work/README.md' } }))
  const length = window.history.length
  await moving(router, () => work.updatePinned('a', 'file', { path: '/work/README.md', view: 'source' }, { replace: true }))
  expect(address()).toBe('/chat/a?panel=file&path=/work/README.md&view=source')
  expect(window.history.length).toBe(length)
  // Back leaves the file, and does not return to its preview.
  expect(work.canGo('a', 'back')).toBe(false)
})

// Planted defect this catches: a panel open on a phone that is no entry of
// the history, which Back leaves with the conversation.
test('on a phone Back from the panel closes it and shows the conversation, as its Back to Conversation does', async () => {
  width.turn(true)
  const { work, router } = await panelPage('a')
  await moving(router, () => work.setOpen('a', true))
  expect(work.stateFor('a').open).toBe(true)
  await moving(router, () => router.back())
  expect(address()).toBe('/chat/a')
  expect(work.stateFor('a').open).toBe(false)

  // The panel's own Back to Conversation goes back to the entry it was opened from.
  await moving(router, () => work.setOpen('a', true))
  const length = window.history.length
  await moving(router, () => work.setOpen('a', false))
  expect([address(), window.history.length, work.stateFor('a').open]).toEqual(['/chat/a', length, false])
})

// Production: a copied link to a file opened on a browser tab the agent had
// shown before: the first read of the tabs applied that show over the link.
// Planted defect this catches: shows of the first read applied over an
// address that names what the panel shows.
test('a show from before the page read the tabs leaves an address that names a file as it is', async () => {
  signIn('one')
  const route = stubPanelRoutes({ revision: 1, tabs: [{ id: 'b1', kind: 'browser', data: { url: 'https://example.test/', tab: 't1', shows: 1 } }] })
  const conversations = useConversations()
  const id = conversations.create()
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  const plugin = (name: string) => ({ id: name, name, description: 'A plugin.', enabled: true, packages: [] })
  useProduct().read = productState({
    conversations: [conversationSummary(id, '')],
    plugins: [plugin('browser'), plugin('file-browser')],
  })
  showMetadata(id, { panel: panelReadSchema.parse({ states: [], ...route.stored }) })
  const work = useWorkPanel()
  const router = attachRouter(work)
  const link = `/chat/${id}?panel=file&path=/work/a.md&view=preview`
  await moving(router, () => void router.push(link))
  work.shown(id)
  work.load(id)
  await waitFor(() => work.stateFor(id).tabsRead)
  await settled()
  expect(address()).toBe(link)
  expect(work.stateFor(id).history.at(-1)).toBe('file')
  // The show counts as applied: it opens its tab neither now nor later.
  expect(readLocalState('one').workPanelShown).toEqual({ [id]: { b1: 1 } })
})

import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'

// The work pane (`WorkPane.vue`) with the real File and Change tabs of their
// plugins in it, over the work store, whose panel a router over happy-dom's
// own history holds, and a Host of files in memory; the panel's routes are
// stubbed. The tabs' components ask for what their views hold beside where
// they are as the panel moves, which a step of the history must survive.
// About 100 ms each, most of it the views' first render.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/chat' })
await import('@demicodes/web-ui/__tests__/vue-components')

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

const { createApp, defineComponent, h, nextTick } = await import('vue')
const { createPinia, disposePinia, setActivePinia } = await import('pinia')
const { createRouter, createWebHistory } = await import('vue-router')
const { z } = await import('zod')
const { waitFor } = await import('@demicodes/utils')
const { emptyChangeSet } = await import('@demicodes/web-ui/files/changes')
const { createMemoryFileSource, dir, file } = await import('@demicodes/web-ui/files/memory-source')
const { createOverlayStore } = await import('@demicodes/web-ui/overlay/overlayStore')
const { providePageHost } = await import('@demicodes/web-ui/plugins/page')
const { useSession } = await import('../auth/session')
const { identitySchema } = await import('../api/generated/web-api')
const { useProduct } = await import('../state/product')
const { conversationSummary, productState } = await import('../__tests__/product-state')
const { useConversations } = await import('./store')
const { useWorkPanel } = await import('./work')
const { default: WorkPane } = await import('./WorkPane.vue')
type PageHost = import('@demicodes/web-ui/plugins/page').PageHost

const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
const fetchDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'fetch')
const saved = new Map<string, string>()
let pinia: ReturnType<typeof createPinia>
const cleanups: (() => void)[] = []

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
  for (const cleanup of cleanups.splice(0).reverse()) {
    cleanup()
  }
  document.body.innerHTML = ''
  disposePinia(pinia)
  for (const [name, descriptor] of [['localStorage', storageDescriptor], ['fetch', fetchDescriptor]] as const) {
    if (descriptor) {
      Object.defineProperty(globalThis, name, descriptor)
    } else {
      Reflect.deleteProperty(globalThis, name)
    }
  }
})

/** The Host's files: a workspace of two documents and a folder. */
function workspace() {
  const at = '2026-09-01T10:00:00Z'
  return createMemoryFileSource({
    platform: 'linux',
    home: '/work',
    root: dir({ work: dir({ 'a.bin': file(4, at), 'b.bin': file(4, at), src: dir({ 'c.bin': file(4, at) }) }) }),
  })
}

/** A host of which the tabs use the files, the intents and the panel, over the work store. */
function pageHost(work: ReturnType<typeof useWorkPanel>): PageHost {
  const unused = () => {
    throw new Error('the test does not use this part of the host')
  }
  const source = workspace()
  return {
    userState: () => undefined,
    followState: unused,
    call: unused,
    stream: unused,
    installed: () => [],
    hostStarting: () => false,
    files: () => ({
      workspace: { source, root: '/work' },
      root: '/work',
      changes: () => emptyChangeSet,
      history: () => null,
      edit: unused,
      request: () => null,
      showChanges: () => {},
    }),
    intents: { open: (conversation, request) => work.openIn(conversation, request), canOpen: (intent) => work.canOpen(intent) },
    panel: {
      tabs: () => [],
      select: (conversation, _kind, id) => work.select(conversation, id),
      canGo: (conversation, direction) => work.canGo(conversation, direction),
      go: (direction) => work.go(direction),
    },
    openSettings: unused,
    openLink: unused,
    overlays: createOverlayStore(),
    pinia: createPinia(),
  }
}

/** The address now, as its user reads it. */
const address = () => decodeURIComponent(window.location.pathname + window.location.search)

/**
 * A page signed in with Files and Changes on, at `/chat/<id>` of a
 * conversation whose panel the backend holds empty, its tabs read and its
 * work pane mounted.
 */
async function panePage() {
  const { user } = identitySchema.parse({ user: {
    id: 'one',
    email: 'one@example.test',
    nickname: 'one',
    role: 'master',
    createdAt: '2026-09-18T00:00:00.000Z',
    disabled: false,
    choosesPassword: false,
  } })
  useSession().current = { status: 'signedIn', user }
  globalThis.fetch = (async (_input: RequestInfo | URL, init?: RequestInit) => {
    // The panel comes with the conversation's entry in the list; only changes are sent.
    return (init?.method ?? 'GET') === 'GET'
      ? Response.json({ code: 'not_found', message: 'No such route' }, { status: 404 })
      : Response.json({ revision: 2, changed: true })
  }) as typeof fetch
  const conversations = useConversations()
  const id = conversations.create()
  conversations.items.find((item) => item.id === id)!.persistence = 'synced'
  const plugin = (name: string) => ({ id: name, name, description: 'A plugin.', enabled: true, packages: [] })
  useProduct().read = productState({
    conversations: [conversationSummary(id, '')],
    plugins: [plugin('changes'), plugin('file-browser')],
  })
  const work = useWorkPanel()
  const router = createRouter({ history: createWebHistory(), routes: [{ path: '/chat/:id?', component: { render: () => null } }] })
  const detach = work.attach(router)
  cleanups.push(() => {
    detach()
    router.options.history.destroy()
  })
  await moving(router, () => void router.push(`/chat/${id}`))
  work.shown(id)
  work.load(id)
  await waitFor(() => work.stateFor(id).tabsRead && z.object({ workPanel: z.literal(true) }).safeParse(window.history.state).success)
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp(defineComponent(() => {
    providePageHost(pageHost(work))
    return () => h(WorkPane, { conversationId: id })
  }))
  app.use(pinia)
  app.mount(root)
  cleanups.push(() => app.unmount())
  await settled()
  return { id, work, router, root }
}

/** The end of the router's next navigation, the store's own or the history's. */
function navigated(router: ReturnType<typeof createRouter>): Promise<void> {
  return new Promise((resolve) => {
    const stop = router.afterEach(() => {
      stop()
      resolve()
    })
  })
}

/** Does `act` and waits for the router to reach where it goes. */
async function moving(router: ReturnType<typeof createRouter>, act: () => void): Promise<void> {
  const arrived = navigated(router)
  act()
  await arrived
}

/** Lets the tabs' components render what the panel shows and ask for what they ask, and the router follow. */
async function settled(): Promise<void> {
  for (let turn = 0; turn < 20; turn++) {
    await nextTick()
    await Promise.resolve()
  }
}

// Production: after A then B opened in Files, Back left the panel; the File
// tab asked to record the view B shows in, in the address's place, which
// cancelled the store's step to B, so B took A's entry. Planted defect this
// catches: a move that replaces the address cancelling a step on its way.
test('a file opened after another in the File view is a step its Back returns from, the tab mounted', async () => {
  const { id, work, router } = await panePage()
  await moving(router, () => work.openIn(id, { intent: 'file', payload: { path: '/work/a.bin' } }))
  await settled()
  const length = window.history.length
  await moving(router, () => work.openIn(id, { intent: 'file', payload: { path: '/work/b.bin' } }))
  await settled()
  expect(address()).toBe(`/chat/${id}?panel=file&path=/work/b.bin&view=preview`)
  expect(window.history.length).toBe(length + 1)

  await moving(router, () => work.go('back'))
  await settled()
  expect(address()).toBe(`/chat/${id}?panel=file&path=/work/a.bin&view=preview`)
  expect(work.stateFor(id).pinned.file).toMatchObject({ path: '/work/a.bin' })
})

test('a commit opened in the Change view is a step its Back returns from, the tab mounted', async () => {
  const { id, work, router } = await panePage()
  await moving(router, () => work.setOpen(id, true))
  await settled()
  expect(address()).toBe(`/chat/${id}?panel=change&view=diff`)
  await moving(router, () => work.openIn(id, { intent: 'commit', payload: { commit: '3f2a9c1' } }))
  await settled()
  expect(address()).toBe(`/chat/${id}?panel=change&commit=3f2a9c1&root=&view=diff`)

  await moving(router, () => work.go('back'))
  await settled()
  expect(address()).toBe(`/chat/${id}?panel=change&view=diff`)
  expect(work.stateFor(id).pinned.change).toMatchObject({ commit: null })
})

// Planted defect this catches: Back to Conversation after several steps
// pushing a closed panel, so the history holds the panel's steps behind the
// conversation and Back opens the panel again.
test('on a phone Back to Conversation after several steps goes back to where the panel opened', async () => {
  width.turn(true)
  const { id, work, router } = await panePage()
  const length = window.history.length
  await moving(router, () => work.openIn(id, { intent: 'file', payload: { path: '/work/a.bin' } }))
  await moving(router, () => work.openIn(id, { intent: 'file', payload: { path: '/work/b.bin' } }))
  await settled()
  expect(window.history.length).toBe(length + 2)

  await moving(router, () => work.setOpen(id, false))
  expect([address(), window.history.length, work.stateFor(id).open]).toEqual([`/chat/${id}`, length + 2, false])
})

// Planted defect this catches: a folder the crumb row goes to selected in the
// view alone, which the address and Back know nothing of.
test('a folder the File view’s crumb row goes to is a step in the address, as a file is', async () => {
  const { id, work, router, root } = await panePage()
  await moving(router, () => work.openIn(id, { intent: 'file', payload: { path: '/work/a.bin' } }))
  await settled()
  const length = window.history.length
  root.querySelector<HTMLElement>('[role="navigation"]')!.click()
  await settled()
  const field = root.querySelector<HTMLInputElement>('input')!
  field.value = '/work/src'
  field.dispatchEvent(new Event('input', { bubbles: true }))
  await settled()
  const arrived = navigated(router)
  field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
  await arrived
  await settled()
  expect(address()).toBe(`/chat/${id}?panel=file&path=/work/a.bin&view=preview&folder=/work/src`)
  expect(window.history.length).toBe(length + 1)

  await moving(router, () => work.go('back'))
  expect(work.stateFor(id).pinned.file).toEqual({ path: '/work/a.bin', view: 'preview' })
})

// The panel draws its frame and its tabs as soon as the conversation is in the
// list, which carries them, and not after its opening
// (`docs/decisions/0049-the-work-panel-shows-at-once.md`). Planted defect this
// catches: the panel held back until the opening brings the conversation's
// facts, which a reload shows as a blank frame. About 100 ms.
test('the panel shows with the conversation in the list, while its opening is still to come', async () => {
  const { id, work, router, root } = await panePage()
  await moving(router, () => work.setOpen(id, true))
  const conversation = useConversations().items.find((item) => item.id === id)!
  const panel = () => root.querySelector('aside')
  const strip = () => [...root.querySelectorAll('[role="tab"]')].map((tab) => tab.textContent?.trim())

  for (const load of ['reading', 'loading', 'opening'] as const) {
    conversation.load = load
    await settled()
    expect(panel()).not.toBeNull()
    expect(strip()).toEqual(['Changes', 'Files'])
  }
})

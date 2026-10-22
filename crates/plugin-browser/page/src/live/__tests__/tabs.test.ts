import { expect, jest, test } from 'bun:test'
import type { LiveModuleMessage, LiveTab, LiveViewerMessage } from '../../generated/plugin'
import { LIVE_CONTROL_FRAME, definePage, type ConversationPlugin, type UserStreamHandlers } from '@demicodes/plugin-sdk'
import { pageContext, type PageHost } from '@demicodes/web-ui/plugins/page'
import { disposePluginStores } from '@demicodes/web-ui/plugins/stores'
import { createPinia } from 'pinia'
import { until } from '@vueuse/core'
import { waitFor } from '@demicodes/utils'
import { computed, effectScope, nextTick, ref, shallowRef } from 'vue'
import {
  BrowserTabsController,
  BrowserTabsError,
  type BrowserTabList,
  type BrowserTabsApi,
  type BrowserTabsOptions,
} from '../tabs'
import { browserStore } from '../store'
import { browserTabsApi } from '../../tabs'

/** A controller in an effect scope of its own, as its store makes it, over a tab list the test sets; what it reports to the user lands in `reported`. */
function harness(
  api: Partial<BrowserTabsApi>,
  options: BrowserTabsOptions = {},
  reported: Array<[string, unknown]> = [],
  defects: Array<[string, unknown]> = [],
) {
  const list = shallowRef<BrowserTabList | null>(null)
  const error = shallowRef<BrowserTabsError | null>(null)
  const scope = effectScope()
  const controller = scope.run(() => new BrowserTabsController(
    {
      tabs: { value: list, error },
      bind: async () => null,
      select: () => {},
      navigate: async () => 0,
      history: async () => 0,
      stop: async () => 0,
      stream: () => ({ send: () => {}, watched: () => {}, close: () => {} }),
      hostStarting: () => false,
      ...api,
    },
    { report: (title, error) => void reported.push([title, error]), defect: (what, error) => void defects.push([what, error]) },
    { visibility: ref<DocumentVisibilityState>('visible'), ...options },
  ))!
  return { controller, list, end: () => { controller.dispose(); scope.stop() } }
}

/** A message of the module, framed as the stream carries it. */
function framed(message: LiveModuleMessage): Uint8Array {
  const json = new TextEncoder().encode(JSON.stringify(message))
  const bytes = new Uint8Array(5 + json.length)
  new DataView(bytes.buffer).setUint32(0, 1 + json.length)
  bytes[4] = LIVE_CONTROL_FRAME
  bytes.set(json, 5)
  return bytes
}

const VIEWPORT = { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' } as const

/** The panel a shown content measures. */
const PANEL = { panel: { width: 800, height: 600 }, devicePixelRatio: 2, screen: { width: 1440, height: 900 } }

/** A view's stream as the test sees it: what the view sent, whether it says anyone watches, and whether it closed. */
interface RecordedView {
  sent: LiveViewerMessage[]
  watched: boolean[]
  closed: boolean
  handlers: UserStreamHandlers
}

/** A stream that keeps what the view sends, as the module reads it. */
function recordingStream(views: RecordedView[]) {
  const decoder = new TextDecoder()
  return (handlers: UserStreamHandlers) => {
    const view: RecordedView = { sent: [], watched: [], closed: false, handlers }
    views.push(view)
    return {
      send: (bytes: Uint8Array) => void view.sent.push(JSON.parse(decoder.decode(bytes.subarray(5))) as LiveViewerMessage),
      watched: (watched: boolean) => void view.watched.push(watched),
      close: () => {
        view.closed = true
      },
    }
  }
}

/**
 * Planted defect this catches: a hidden page that closes its view, which
 * shows nothing until a new one connects, or keeps it watched, which keeps
 * the conversation awake.
 */
test('a hidden page keeps its view unwatched, and shown again watches the shown tab on the same view', async () => {
  const visibility = ref<DocumentVisibilityState>('visible')
  const views: RecordedView[] = []
  const { controller } = harness({ stream: recordingStream(views) }, { visibility, pictures: async () => true })
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  expect(views).toHaveLength(1)
  // Nobody can watch a hidden page: the Host stops capturing, and the stream keeps nothing awake.
  visibility.value = 'hidden'
  expect(views[0]!.closed).toBe(false)
  expect(views[0]!.sent.at(-1)).toEqual({ type: 'watch', tab: null })
  expect(views[0]!.watched).toEqual([false])
  // Another tab shown meanwhile watches nothing either.
  controller.hide('p-t1')
  controller.show('p-t2', 't2')
  expect(views[0]!.sent.at(-1)).toEqual({ type: 'watch', tab: null })
  visibility.value = 'visible'
  expect(views).toHaveLength(1)
  expect(views[0]!.watched).toEqual([false, true])
  expect(views[0]!.sent.at(-1)).toEqual({ type: 'watch', tab: 't2' })
  controller.dispose()
  expect(views[0]!.closed).toBe(true)
})

const KEPT_TAB: LiveTab = { id: 't1', title: 'Orders', url: 'https://example.test/orders', createdBy: { kind: 'user' }, viewport: VIEWPORT, loading: false, canGoBack: false, canGoForward: false }

/** A page host for the browser's store: the plugin's state, calls and stream over the test's, its Pinia, and nothing else. */
function storeHost(views: RecordedView[], list: BrowserTabList): PageHost {
  const unused = () => {
    throw new Error('the test does not use this part of the host')
  }
  return {
    userState: () => undefined,
    followState: () => ({ value: () => list, error: () => null, read: () => {}, stop: () => {} }),
    call: unused,
    stream: () => recordingStream(views),
    installed: () => [],
    hostStarting: () => false,
    files: unused,
    intents: { open: unused, canOpen: () => false },
    panel: { tabs: () => [], select: () => {}, canGo: () => false, go: () => {} },
    openSettings: unused,
    openLink: unused,
    overlays: { open: unused } as never,
    pinia: createPinia(),
  }
}

/**
 * Planted defect this catches: a conversation's browser that ends with its
 * panel, as the panel session did, which drops the picture and opens a new
 * connection on return.
 */
test('a return to a conversation shows its tab’s kept picture at once, on the connection it kept', async () => {
  const views: RecordedView[] = []
  const list: BrowserTabList = { tabs: [] }
  const host = storeHost(views, list)
  const page = pageContext(host, definePage({ plugin: 'browser' }))
  const store = browserStore({ visibility: ref<DocumentVisibilityState>('visible'), pictures: async () => true })
  const browsers = page.store(store)

  // The panel shows the conversation's tab: its content watches the conversation's browser.
  const shown = browsers.watch('c1')
  const tabs = shown.session.tabs
  await until(tabs.pictures).toBe('supported')
  tabs.resize(PANEL)
  tabs.show('p-t1', 't1')
  views[0]!.handlers.data(framed({ type: 'state', running: true, list: 2, tabs: [KEPT_TAB], watched: 't1' }))
  const picture = { image: { width: 1600, height: 1200 } as never, stream: { tab: 't1', generation: 1, width: 1600, height: 1200, viewport: VIEWPORT, scale: 1 } }
  // The user switches to another conversation: the content unmounts, handing on its picture.
  tabs.keepPicture('t1', picture)
  tabs.hide('p-t1')
  shown.end()
  await waitFor(() => views[0]!.sent.at(-1)?.type === 'watch' && views[0]!.watched.length > 0)
  expect(views[0]!.sent.at(-1)).toEqual({ type: 'watch', tab: null })
  expect(views[0]!.watched).toEqual([false])
  browsers.watch('c2').end()

  // And back: the same browser, its picture at once, and the same connection watching again.
  const back = browsers.watch('c1')
  expect(back.session.tabs).toBe(tabs)
  expect(back.session.tabs.keptPicture('t1')).toBe(picture)
  back.session.tabs.show('p-t1', 't1')
  expect(views).toHaveLength(1)
  expect(views[0]!.closed).toBe(false)
  expect(views[0]!.watched).toEqual([false, true])
  expect(views[0]!.sent.at(-1)).toEqual({ type: 'watch', tab: 't1' })
  back.end()
  // Turned off, the plugin's store ends the connection.
  disposePluginStores(host.pinia, 'browser')
  expect(views[0]!.closed).toBe(true)
})

test('a view waiting to reconnect connects at once when a tab that just got its browser tab shows', async () => {
  const opened: UserStreamHandlers[] = []
  const { controller } = harness(
    {
      stream: (handlers) => {
        opened.push(handlers)
        return { send: () => {}, watched: () => {}, close: () => {} }
      },
    },
    { pictures: async () => true },
  )
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  // The Cloud ran no browser yet, so the view ended; its next try is a wait away.
  opened[0]!.closed('host_stopped')
  expect(opened).toHaveLength(1)
  // A tab that just got its browser tab: the view does not wait.
  controller.show('p-t2', 't2')
  expect(opened).toHaveLength(2)
  controller.dispose()
})

test('a view opens once the shown content measured its panel, names the panel before the tab, and hears of a resize once it settles', async () => {
  jest.useFakeTimers()
  try {
    const views: RecordedView[] = []
    const { controller, end } = harness({ stream: recordingStream(views) }, { pictures: async () => true })
    await until(controller.pictures).toBe('supported')
    // A capture starts at the panel's size, which nobody knows before the content measures it.
    controller.show('p-t1', 't1')
    expect(views).toHaveLength(0)
    controller.resize(PANEL)
    expect(views[0]!.sent).toEqual([
      { type: 'hello', platform: expect.any(String) },
      { type: 'panel', width: 800, height: 600, devicePixelRatio: 2, screenWidth: 1440, screenHeight: 900 },
      { type: 'watch', tab: 't1' },
    ])
    // A drag resizes the panel many times; the module hears of the size it settles at.
    for (const width of [790, 780, 770]) {
      controller.resize({ ...PANEL, panel: { width, height: 600 } })
      jest.advanceTimersByTime(50)
    }
    expect(views[0]!.sent.filter((message) => message.type === 'panel')).toHaveLength(1)
    jest.advanceTimersByTime(100)
    expect(views[0]!.sent.at(-1)).toMatchObject({ type: 'panel', width: 770 })
    // Another tab shown in the same view is the only watch the view hears: nothing watches nothing between them.
    controller.hide('p-t1')
    controller.show('p-t2', 't2')
    expect(views[0]!.sent.filter((message) => message.type === 'watch')).toEqual([
      { type: 'watch', tab: 't1' },
      { type: 'watch', tab: 't2' },
    ])
    end()
  } finally {
    jest.useRealTimers()
  }
})

/** A view open on `t1`, whose module the test speaks for, and the answers to the user's requests, which the test gives. */
async function requestHarness() {
  const opened: UserStreamHandlers[] = []
  const answers: Array<{ resolve: (list: number) => void; reject: (error: unknown) => void }> = []
  const answer = () => new Promise<number>((resolve, reject) => void answers.push({ resolve, reject }))
  const { controller, end } = harness(
    {
      stream: (handlers) => {
        opened.push(handlers)
        return { send: () => {}, watched: () => {}, close: () => {} }
      },
      navigate: answer,
      history: answer,
      stop: answer,
    },
    { pictures: async () => true },
  )
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  /** The module's tab list numbered `list`, with `t1` loading or not. */
  const report = (list: number, loading: boolean, url = 'https://example.test/orders') =>
    opened[0]!.data(framed({
      type: 'state',
      running: true,
      list,
      tabs: [{ id: 't1', title: 'Orders', url, createdBy: { kind: 'user' }, viewport: VIEWPORT, loading, canGoBack: true, canGoForward: false }],
      watched: 't1',
    }))
  report(4, false)
  // The tab shows its picture, as a page that loaded does.
  opened[0]!.data(framed({ type: 'stream', tab: 't1', generation: 1, width: 1600, height: 1200, viewport: VIEWPORT, scale: 1 }))
  controller.session.value!.showed({ tab: 't1', generation: 1, width: 1600, height: 1200, viewport: VIEWPORT, scale: 1 })
  return { controller, answers, report, opened, end }
}

/** The panel tab `p1`, bound to `t1`, as its data says. */
const ORDERS = { url: 'https://example.test/orders', tab: 't1' }

test('Reload shows the tab loading in the same call, Stop keeps it loading until a list numbered after its answer says it stopped', async () => {
  const { controller, answers, report, end } = await requestHarness()
  expect(controller.busy('p1', ORDERS)).toBe(false)
  void controller.history('t1', 'reload')
  // Before anything left the page: Stop, the strip's spinner and the line read this one state.
  expect(controller.busy('p1', ORDERS)).toBe(true)
  answers[0]!.resolve(4)
  report(5, true)
  const stopped = controller.stop('t1')
  expect(controller.busy('p1', ORDERS)).toBe(true)
  answers[1]!.resolve(5)
  await stopped
  // A list the Host read before the Stop still describes the page loading.
  report(5, true)
  expect(controller.busy('p1', ORDERS)).toBe(true)
  report(6, false)
  expect(controller.busy('p1', ORDERS)).toBe(false)
  end()
})

test('a Reload shows the page loading from the click until a list numbered after its answer says it stopped', async () => {
  const { controller, answers, report, end } = await requestHarness()
  const url = 'https://example.test/orders'
  expect(controller.loading('t1', url)).toBe(false)
  const reloaded = controller.history('t1', 'reload')
  // Before anything left the page.
  expect(controller.loading('t1', url)).toBe(true)
  // A list the Host read before the request started still describes the page as it was.
  report(5, false)
  expect(controller.loading('t1', url)).toBe(true)
  answers[0]!.resolve(5)
  await reloaded
  expect(controller.loading('t1', url)).toBe(true)
  // A far browser starts loading after its answer, and says so.
  report(6, true)
  expect(controller.loading('t1', url)).toBe(true)
  report(7, false)
  expect(controller.loading('t1', url)).toBe(false)
  end()
})

test('a fast page whose lists reach the page before the answer ends the loading with the answer', async () => {
  const { controller, answers, report, end } = await requestHarness()
  const url = 'https://example.test/docs'
  const navigated = controller.navigate('t1', url)
  // The stream outran the call: the page loaded, and both lists came first.
  report(5, true, url)
  report(6, false, url)
  expect(controller.loading('t1', url)).toBe(true)
  answers[0]!.resolve(4)
  await navigated
  expect(controller.loading('t1', url)).toBe(false)
  end()
})

test('a refused Back ends the loading at once and rejects with why, and a replaced request refused late changes nothing', async () => {
  const { controller, answers, end } = await requestHarness()
  const url = 'https://example.test/orders'
  const back = controller.history('t1', 'back')
  expect(controller.loading('t1', url)).toBe(true)
  answers[0]!.reject(BrowserTabsError.of('history_boundary'))
  expect(await back.catch((error: BrowserTabsError) => error.reason)).toBe('history_boundary')
  expect(controller.loading('t1', url)).toBe(false)
  const forward = controller.history('t1', 'forward')
  void controller.history('t1', 'reload')
  answers[1]!.reject(BrowserTabsError.of('history_boundary'))
  await forward.catch(() => {})
  expect(controller.loading('t1', url)).toBe(true)
  end()
})

/** A controller whose plugin binds with `bind`, and the views it opened, whose module the test speaks for. */
async function openingHarness(bind: () => Promise<string | null>) {
  const opened: UserStreamHandlers[] = []
  const { controller, end } = harness(
    {
      bind,
      stream: (handlers) => {
        opened.push(handlers)
        return { send: () => {}, watched: () => {}, close: () => {} }
      },
    },
    { pictures: async () => true },
  )
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  return { controller, opened, end }
}

test('a reopened tab loads until its first picture, though the list already says it stopped loading', async () => {
  const { controller, opened, end } = await openingHarness(async () => 't2')
  const reopened = { url: 'https://example.test/', tab: 't2' }
  await controller.bind('p1')
  controller.show('p-t2', 't2')
  opened[0]!.data(framed({
    type: 'state',
    running: true,
    list: 3,
    tabs: [{ id: 't2', title: 'Example', url: 'https://example.test/', createdBy: { kind: 'user' }, viewport: VIEWPORT, loading: false, canGoBack: false, canGoForward: false }],
    watched: 't2',
  }))
  // No blank tab with Reload: no picture has shown yet.
  expect(controller.busy('p1', reopened)).toBe(true)
  opened[0]!.data(framed({ type: 'stream', tab: 't2', generation: 1, width: 1600, height: 1200, viewport: VIEWPORT, scale: 1 }))
  controller.session.value!.showed({ tab: 't2', generation: 1, width: 1600, height: 1200, viewport: VIEWPORT, scale: 1 })
  expect(controller.busy('p1', reopened)).toBe(false)
  end()
})

test('a Reload the lost browser answered ends on the new browser’s first list, numbered after it', async () => {
  const { controller, answers, opened, end } = await requestHarness()
  const url = 'https://example.test/orders'
  const reloaded = controller.history('t1', 'reload')
  answers[0]!.resolve(27)
  await reloaded
  // The browser ends; the view opens again on the conversation's next browser.
  opened[0]!.closed('browser_lost')
  controller.session.value!.reconnect()
  const tab = { id: 't1', title: 'Orders', url, createdBy: { kind: 'user' } as const, viewport: VIEWPORT, loading: false, canGoBack: false, canGoForward: false }
  opened[1]!.data(framed({ type: 'state', running: true, list: 28, tabs: [tab], watched: 't1' }))
  expect(controller.loading('t1', url)).toBe(false)
  end()
})

test('a view the stopped Cloud refused connects at once once the Cloud runs', async () => {
  const starting = ref(true)
  const opened: UserStreamHandlers[] = []
  const { controller, end } = harness(
    {
      hostStarting: () => starting.value,
      stream: (handlers) => {
        opened.push(handlers)
        return { send: () => {}, watched: () => {}, close: () => {} }
      },
    },
    { pictures: async () => true },
  )
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p1', null)
  opened[0]!.closed('host_stopped')
  expect(opened).toHaveLength(1)
  // The Cloud runs: the view does not wait out its reconnect wait.
  starting.value = false
  await nextTick()
  expect(opened).toHaveLength(2)
  end()
})

// A call whose caller goes away, as a panel that closes, is dropped and
// never sent.
test('a call still waiting for the backend when its browser is released is dropped with the abort', async () => {
  const waiting: AbortSignal[] = []
  const plugin: ConversationPlugin = {
    state: () => ({ value: computed(() => null), error: computed(() => null), read: () => {} }),
    // The shell holds the call while it cannot reach the backend, until its signal aborts.
    call: (_method, _params, _result, options) =>
      new Promise((_resolve, reject) => {
        const signal = options!.signal!
        waiting.push(signal)
        signal.addEventListener('abort', () => reject(signal.reason), { once: true })
      }),
    stream: () => recordingStream([]),
    installed: computed(() => []),
    hostStarting: computed(() => false),
  }
  const session = effectScope()
  const api = session.run(() => browserTabsApi(plugin, () => {}))!
  const reload = api.history('t1', 'reload')
  expect(waiting.map((signal) => signal.aborted)).toEqual([false])
  session.stop()
  await expect(reload).rejects.toMatchObject({ name: 'AbortError' })
})

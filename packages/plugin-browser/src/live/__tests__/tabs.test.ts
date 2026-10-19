import { expect, jest, test } from 'bun:test'
import type { LiveModuleMessage, LiveTab, LiveViewerMessage } from '../../generated/plugin'
import { LIVE_CONTROL_FRAME, LIVE_VIDEO_CODEC, PluginCallError, type ConversationPlugin, type UserStreamHandlers } from '@demicodes/plugin-sdk'
import { until } from '@vueuse/core'
import { waitFor } from '@demicodes/utils'
import { createPageI18n } from '@demicodes/web-ui/i18n/text'
import { computed, effectScope, nextTick, ref, shallowRef } from 'vue'
import {
  BrowserTabsController,
  BrowserTabsError,
  type BrowserTabList,
  type BrowserTabsApi,
  type BrowserTabsOptions,
  type PictureSupport,
} from '../tabs'
import { browserTabKind } from '../kind'
import { browserTabsApi } from '../../tabs'

/** A controller in a panel session's effect scope, over a tab list the test sets; what it reports to the user lands in `reported`. */
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
      stream: () => ({ send: () => {}, close: () => {} }),
      hostStarting: () => false,
      ...api,
    },
    { report: (title, error) => void reported.push([title, error]), defect: (what, error) => void defects.push([what, error]) },
    { visibility: ref<DocumentVisibilityState>('visible'), ...options },
  ))!
  return { controller, list, end: () => { controller.dispose(); scope.stop() } }
}

/**
 * The web browser's `VideoDecoder`, answering by codec, until the returned
 * function puts back its absence: bun has no WebCodecs.
 */
function stubDecoder(decodes: (codec: string) => boolean): () => void {
  Object.defineProperty(globalThis, 'VideoDecoder', {
    configurable: true,
    value: class {
      static async isConfigSupported(config: VideoDecoderConfig): Promise<VideoDecoderSupport> {
        return { supported: decodes(config.codec), config }
      }
    },
  })
  return () => void Reflect.deleteProperty(globalThis, 'VideoDecoder')
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

/** A stream that keeps what the view sends, as the module reads it. */
function recordingStream(views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }>) {
  const decoder = new TextDecoder()
  return (handlers: UserStreamHandlers) => {
    const view = { sent: [] as LiveViewerMessage[], closed: false, handlers }
    views.push(view)
    return {
      send: (bytes: Uint8Array) => void view.sent.push(JSON.parse(decoder.decode(bytes.subarray(5))) as LiveViewerMessage),
      close: () => {
        view.closed = true
      },
    }
  }
}

test('a notice the picture shows through is a toast in the page’s words, once; one that leaves no picture is none', async () => {
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const reported: Array<[string, unknown]> = []
  const { controller, end } = harness({ stream: recordingStream(views) }, { pictures: async () => true }, reported)
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  views[0]!.handlers.data(framed({ type: 'notice', code: 'driver_error', message: 'Input.dispatchKeyEvent: target closed' }))
  views[0]!.handlers.data(framed({ type: 'notice', code: 'capture_unavailable', message: 'this CPU reports SME without SVE' }))
  expect(reported).toHaveLength(1)
  const [title, error] = reported[0]!
  expect(title).toBe('Could Not Operate the Browser')
  expect(error).toMatchObject({ reason: 'driver_error', message: 'The browser on the device ran into a problem.' })
  expect(controller.session.value?.state.pictureless).toBe('capture_unavailable')
  end()
})

test('a hidden page closes its view, and shown again watches the shown tab on a new view', async () => {
  const visibility = ref<DocumentVisibilityState>('visible')
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const { controller } = harness({ stream: recordingStream(views) }, { visibility, pictures: async () => true })
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  expect(views).toHaveLength(1)
  // Nobody can watch a hidden page, and an open view would keep its Cloud awake.
  visibility.value = 'hidden'
  expect(views[0]!.closed).toBe(true)
  expect(controller.session.value).toBeNull()
  // Another tab shown meanwhile opens nothing either.
  controller.hide('p-t1')
  controller.show('p-t2', 't2')
  expect(views).toHaveLength(1)
  visibility.value = 'visible'
  expect(views).toHaveLength(2)
  expect(views[1]!.sent.map((message) => message.type)).toEqual(['hello', 'panel', 'watch'])
  expect(views[1]!.sent.at(-1)).toEqual({ type: 'watch', tab: 't2' })
  controller.dispose()
  expect(views[1]!.closed).toBe(true)
})

test('a view waiting to reconnect connects at once when a tab that just got its browser tab shows', async () => {
  const opened: UserStreamHandlers[] = []
  const { controller } = harness(
    {
      stream: (handlers) => {
        opened.push(handlers)
        return { send: () => {}, close: () => {} }
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

test('a web browser that cannot decode the pictures opens no view, and one that can opens one', async () => {
  // A Chromium built without proprietary codecs has WebCodecs, and VP8 and VP9, but no H.264.
  // The page asks for the codec the Host's extension encodes, the live protocol's.
  const webBrowsers: Array<[string, ((codec: string) => boolean) | null, PictureSupport, number]> = [
    ['no WebCodecs', null, 'unsupported', 0],
    ['WebCodecs without H.264', (codec) => codec.startsWith('vp'), 'unsupported', 0],
    ['WebCodecs with H.264', (codec) => codec === LIVE_VIDEO_CODEC, 'supported', 1],
  ]
  for (const [webBrowser, decodes, support, viewCount] of webBrowsers) {
    const restore = decodes ? stubDecoder(decodes) : () => {}
    try {
      let views = 0
      const { controller } = harness({
        stream: () => {
          views += 1
          return { send: () => {}, close: () => {} }
        },
      })
      controller.resize(PANEL)
      controller.show('p-t1', 't1')
      await until(controller.pictures).not.toBe('checking')
      expect({ webBrowser, support: controller.pictures.value, views }).toEqual({ webBrowser, support, views: viewCount })
      controller.dispose()
    } finally {
      restore()
    }
  }
})

test('a view opens once the shown content measured its panel, names the panel before the tab, and hears of a resize once it settles', async () => {
  jest.useFakeTimers()
  try {
    const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
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

test('a tab shown again shows what the browser last said of it, though no view is open', async () => {
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const visibility = ref<DocumentVisibilityState>('visible')
  const { controller, end } = harness({ stream: recordingStream(views) }, { visibility, pictures: async () => true })
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  const tab = { id: 't1', title: 'Orders', url: 'https://example.test/orders', createdBy: { kind: 'user' } as const, viewport: VIEWPORT, loading: false, canGoBack: false, canGoForward: false }
  views[0]!.handlers.data(framed({ type: 'state', running: true, list: 1, tabs: [tab], watched: 't1' }))
  // The user's browser hides the page: the view closes, and the tab keeps its address and that it loaded.
  visibility.value = 'hidden'
  expect(controller.session.value).toBeNull()
  expect(controller.tab('t1')).toEqual(tab)
  // A view that reports the browser without it lets it go.
  visibility.value = 'visible'
  views[1]!.handlers.data(framed({ type: 'state', running: true, list: 1, tabs: [], watched: null }))
  expect(controller.tab('t1')).toBeNull()
  end()
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
        return { send: () => {}, close: () => {} }
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

test('the agent’s navigation shows the tab loading while the tab list says it loads', async () => {
  const { controller, report, end } = await requestHarness()
  report(5, true)
  expect(controller.busy('p1', ORDERS)).toBe(true)
  report(6, false)
  expect(controller.busy('p1', ORDERS)).toBe(false)
  end()
})

// `live-view.md` § The tab methods: the panel says every failure in its own
// words. The user saw "The browser couldn't do that." where the browser did
// not start on the device; a failure that carries no reason of the plugin's
// says whether the server answered, and with what, or that its answer could
// not be read.
test('a failed call says its reason’s sentence, or how the server answered, or that it did not or could not be read', async () => {
  const failures: unknown[] = [
    new PluginCallError('browser_unavailable', 'native artifact: the artifact has 60 bytes', { type: 'status', status: '409 Conflict' }),
    new PluginCallError('plugin_failed', 'the browser answered what the plugin cannot read', { type: 'status', status: '500 Internal Server Error' }),
    new PluginCallError('unreadable_answer', 'Invalid server response: not JSON', { type: 'unreadable' }),
    new PluginCallError('failed', 'Failed to fetch'),
    new TypeError('Failed to fetch'),
  ]
  const plugin: ConversationPlugin = {
    state: () => ({ value: computed(() => null), error: computed(() => null), read: () => {} }),
    call: async () => {
      throw failures.shift()
    },
    stream: () => recordingStream([]),
    installed: computed(() => []),
    hostStarting: computed(() => false),
  }
  const session = effectScope()
  const api = session.run(() => browserTabsApi(plugin, () => {}))!
  const said: Array<[string | null, string]> = []
  for (let call = 0; call < 5; call += 1) {
    await api.navigate('t1', 'https://example.test/').catch((error: BrowserTabsError) => void said.push([error.reason, error.message]))
  }
  expect(said).toEqual([
    ['browser_unavailable', 'The browser couldn’t start on the device.'],
    [null, 'The server couldn’t do that (500 Internal Server Error).'],
    [null, 'The server’s answer couldn’t be read.'],
    [null, 'The server didn’t answer.'],
    [null, 'The server didn’t answer.'],
  ])
  session.stop()
})

test('Retry of a tab that could not open loads in the same call, and a failure that comes back ends it', async () => {
  let answer = (_tab: string | null) => {}
  const { controller, end } = harness({ bind: () => new Promise<string | null>((resolve) => (answer = resolve)) })
  const failed = { url: 'https://example.test/', failure: { code: 'device_offline' as const, message: 'Offline' } }
  expect(controller.busy('p1', failed)).toBe(false)
  const retried = controller.bind('p1')
  // The failure gives way before the plugin heard of the Retry.
  expect(controller.opening('p1', failed)).toBe(true)
  expect(controller.busy('p1', failed)).toBe(true)
  answer(null)
  await retried
  expect(controller.busy('p1', failed)).toBe(false)
  end()
})

test('a lost tab shown loads from the ask until its data names the tab the plugin opened, whichever comes first', async () => {
  let answer = (_tab: string | null) => {}
  const { controller, end } = harness({ bind: () => new Promise<string | null>((resolve) => (answer = resolve)) })
  const lost = { url: 'https://example.test/', tab: 't1', closed: true }
  const reopened = { url: 'https://example.test/', tab: 't2' }
  expect(controller.busy('p1', lost)).toBe(false)
  const bound = controller.bind('p1')
  expect(controller.busy('p1', lost)).toBe(true)
  // The answer outran the panel's data: the tab still loads.
  answer('t2')
  await bound
  expect(controller.busy('p1', lost)).toBe(true)
  // Its data names the new tab, which no view reported yet: it opens.
  controller.show('p-t2', 't2')
  expect(controller.busy('p1', reopened)).toBe(true)
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

test('a tab opened in the background takes its page’s title once the browser names it, and one its user sends elsewhere never the old page’s', async () => {
  const { controller, answers, opened, end } = await requestHarness()
  const title = (data: { url: string; tab?: string; title?: string }) => browserTabKind.title(data, { conversation: 'c1', session: controller })
  const tabs = (list: number, invoice: string, ordersLoading = false) => opened[0]!.data(framed({
    type: 'state',
    running: true,
    list,
    tabs: [
      { id: 't1', title: 'Orders', url: 'https://example.test/orders', createdBy: { kind: 'user' }, viewport: VIEWPORT, loading: ordersLoading, canGoBack: true, canGoForward: false },
      { id: 't2', title: invoice, url: 'https://example.test/invoice', createdBy: { kind: 'user' }, viewport: VIEWPORT, loading: invoice === '', canGoBack: false, canGoForward: false },
    ],
    watched: 't1',
  }))
  // A tab the page opened in the background: the panel tab has its address and browser tab, and is never shown.
  const invoice = { url: 'https://example.test/invoice', tab: 't2' }
  expect(title(invoice)).toBe('example.test')
  tabs(5, '')
  expect(title(invoice)).toBe('example.test')
  tabs(6, 'Invoice 42')
  expect(title(invoice)).toBe('Invoice 42')
  // The user sends the shown tab elsewhere: its tab is named after where it goes, until the browser names that.
  const navigated = controller.navigate('t1', 'https://example.test/docs')
  expect(title({ url: 'https://example.test/docs', tab: 't1' })).toBe('example.test')
  answers[0]!.resolve(6)
  await navigated
  tabs(6, 'Invoice 42')
  expect(title({ url: 'https://example.test/docs', tab: 't1' })).toBe('example.test')
  // The browser loads, still on the page it leaves until the new one commits.
  tabs(7, 'Invoice 42', true)
  expect(title({ url: 'https://example.test/docs', tab: 't1' })).toBe('example.test')
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

// After Demi restarts, the runner stops the browser and the tabs are lost
// with it: a Reload sent then is refused with tab_not_found, and the tab
// opens again on its address instead (`live-view.md` § A browser tab in the
// panel).
test('a request on a tab the browser lost says nothing: the browser tells the plugin', async () => {
  const answers: Array<{ reject: (error: unknown) => void }> = []
  const reported: Array<[string, unknown]> = []
  const defects: Array<[string, unknown]> = []
  const { controller, end } = harness(
    { history: () => new Promise<number>((_resolve, reject) => void answers.push({ reject })) },
    { pictures: async () => true },
    reported,
    defects,
  )
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  const reload = controller.history('t1', 'reload')
  answers[0]!.reject(BrowserTabsError.of('tab_not_found'))
  await reload
  expect(reported).toEqual([])
  expect(defects).toEqual([])
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
        return { send: () => {}, close: () => {} }
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

test('a tab a page opens joins the strip at once, selected when the user’s click in the watched tab opened it', async () => {
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const selected: string[] = []
  const { controller, end } = harness(
    { stream: recordingStream(views), select: (panelTab) => void selected.push(panelTab) },
    { pictures: async () => true },
  )
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  const tab = (id: string, createdBy: LiveTab['createdBy']): LiveTab => ({
    id, title: id, url: `https://example.test/${id}`, createdBy, viewport: VIEWPORT,
    loading: false, canGoBack: false, canGoForward: false,
  })
  const state = (list: number, tabs: LiveTab[]) =>
    views[0]!.handlers.data(framed({ type: 'state', running: true, list, tabs, watched: 't1' }))
  const watchedTab = tab('t1', { kind: 'agent', number: 1 })
  state(1, [watchedTab])
  // A page that opens a tab by itself, as a timer's window.open does: the strip has it at once, not selected.
  state(2, [watchedTab, tab('t2', { kind: 'page', opener: 't1' })])
  expect(selected).toEqual([])
  // The user's click on a target=_blank link: the browser selects the tab it opens.
  controller.session.value!.input({
    type: 'input', tab: 't1', input: { type: 'pointer', action: 'down', x: 10, y: 10, button: 'left', buttons: 1, clickCount: 1, modifiers: 0 },
  })
  state(3, [watchedTab, tab('t2', { kind: 'page', opener: 't1' }), tab('t3', { kind: 'page', opener: 't1' })])
  expect(selected).toEqual(['browser-t3'])
  // A list that names no new tab selects nothing more.
  state(4, [watchedTab, tab('t2', { kind: 'page', opener: 't1' }), tab('t3', { kind: 'page', opener: 't1' })])
  expect(selected).toEqual(['browser-t3'])
  end()
})

test('the user’s downloads in a tab stay listed while its view opens again', async () => {
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const { controller, end } = harness({ stream: recordingStream(views) }, { pictures: async () => true })
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  const report = { id: 'g1', name: 'report.pdf', state: 'complete', received: 48, total: 48, path: '/tmp/downloads/report.pdf' } as const
  views[0]!.handlers.data(framed({ type: 'downloads', tab: 't1', downloads: [report] }))
  expect(controller.downloadsOf('t1')).toEqual([report])
  expect(controller.downloadsOf('t2')).toEqual([])
  // A lost connection ends the view; the bubble keeps what it showed.
  views[0]!.handlers.closed('lost')
  expect(controller.downloadsOf('t1')).toEqual([report])
  end()
})

test('a view’s state replaces the downloads the page holds: only those of the tab it watches stay', async () => {
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const { controller, end } = harness({ stream: recordingStream(views) }, { pictures: async () => true })
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t1', 't1')
  const download = (id: string) => ({ id, name: `${id}.pdf`, state: 'complete', received: 48, total: 48, path: `/tmp/downloads/${id}.pdf` }) as const
  const tab = (id: string): LiveTab => ({
    id, title: id, url: `https://example.test/${id}`, createdBy: { kind: 'agent', number: 1 }, viewport: VIEWPORT,
    loading: false, canGoBack: false, canGoForward: false,
  })
  const state = (list: number, tabs: LiveTab[]) =>
    views[0]!.handlers.data(framed({ type: 'state', running: true, list, tabs, watched: 't1' }))
  views[0]!.handlers.data(framed({ type: 'downloads', tab: 't1', downloads: [download('g1')] }))
  views[0]!.handlers.data(framed({ type: 'downloads', tab: 't2', downloads: [download('g2')] }))
  views[0]!.handlers.data(framed({ type: 'downloads', tab: 't3', downloads: [download('g3')] }))
  // The state lists t1 and t2 and the page watches t1: t3 is gone, and t2 is not watched.
  state(1, [tab('t1'), tab('t2')])
  expect(controller.downloadsOf('t1')).toEqual([download('g1')])
  expect(controller.downloadsOf('t2')).toEqual([])
  expect(controller.downloadsOf('t3')).toEqual([])
  // The watched tab itself went.
  state(2, [tab('t2')])
  expect(controller.downloadsOf('t1')).toEqual([])
  end()
})

test('before the browser has a tab, the content says what Demi does: connecting, the Cloud, the browser, then the page', async () => {
  const starting = ref(false)
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const { controller, list, end } = harness(
    { hostStarting: () => starting.value, stream: recordingStream(views) },
    { pictures: async () => true },
  )
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  const opening = { url: 'https://example.test/' }
  // A content shown before the browser has its tab: the view tells the Host the panel, and watches nothing.
  controller.show('p1', null)
  expect(views.map((view) => view.sent.map((message) => message.type))).toEqual([['hello', 'panel']])
  // Neither the plugin's list nor the view said yet what the Host is doing.
  expect(controller.startingPhase('p1', opening)).toBe('connecting')
  // The product state says the Cloud starts, which comes first.
  starting.value = true
  expect(controller.startingPhase('p1', opening)).toBe('cloud')
  starting.value = false
  expect(controller.startingPhase('p1', opening)).toBe('connecting')
  // The conversation's browser has no tabs yet, as the plugin's list says until the view says otherwise.
  list.value = { tabs: [] }
  expect(controller.startingPhase('p1', opening)).toBe('browser')
  const tab = (id: string) => ({ id, title: id, url: 'about:blank', createdBy: { kind: 'user' as const }, viewport: VIEWPORT, loading: false, canGoBack: false, canGoForward: false })
  views[0]!.handlers.data(framed({ type: 'state', running: true, list: 1, tabs: [tab('t1')], watched: null }))
  expect(controller.startingPhase('p1', opening)).toBe('page')
  // The browser has the tab once its list names it: the same view watches it, and the content shows the page.
  const bound = { url: 'https://example.test/', tab: 't2' }
  views[0]!.handlers.data(framed({ type: 'state', running: true, list: 2, tabs: [tab('t1'), tab('t2')], watched: null }))
  expect(controller.startingPhase('p1', bound)).toBeNull()
  controller.show('p1', 't2')
  expect(views).toHaveLength(1)
  expect(views[0]!.sent.at(-1)).toEqual({ type: 'watch', tab: 't2' })
  // A tab lost with the browser waits for it again; one that could not open does not, until its Retry.
  expect(controller.startingPhase('p1', { ...bound, closed: true })).toBe('page')
  expect(controller.startingPhase('p1', { ...opening, failure: { code: 'device_offline' as const, message: 'Offline' } })).toBeNull()
  end()
})

test('a reopening tab its user stopped is blank, waiting for nothing; an address entered then loads and says what it waits for', async () => {
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  let answer = (_tab: string | null) => {}
  const { controller, list, end } = harness(
    { stream: recordingStream(views), bind: () => new Promise<string | null>((resolve) => (answer = resolve)) },
    { pictures: async () => true },
  )
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  // A tab the browser lost, shown: it opens again, and its user pressed Stop before the browser had it.
  const lost = { tab: 't0', closed: true }
  const blank = { ...lost, url: 'about:blank' }
  controller.show('p1', null)
  const bound = controller.bind('p1')
  // Nothing said yet what the Host does, then the browser does not run.
  expect(controller.startingPhase('p1', blank)).toBeNull()
  expect(controller.busy('p1', blank)).toBe(false)
  list.value = { tabs: [] }
  views[0]!.handlers.data(framed({ type: 'state', running: false, list: 1, tabs: [], watched: null }))
  expect(controller.startingPhase('p1', blank)).toBeNull()
  expect(controller.busy('p1', blank)).toBe(false)
  // The user enters an address before the browser has the tab: it loads, and says what it waits for.
  const asked = { ...lost, url: 'https://example.test/' }
  expect(controller.startingPhase('p1', asked)).toBe('browser')
  expect(controller.busy('p1', asked)).toBe(true)
  // The browser opened the blank tab: shown before its first picture, it still loads nothing.
  const tab = { id: 't1', title: '', url: 'about:blank', createdBy: { kind: 'user' as const }, viewport: VIEWPORT, loading: false, canGoBack: false, canGoForward: false }
  views[0]!.handlers.data(framed({ type: 'state', running: true, list: 2, tabs: [tab], watched: null }))
  answer('t1')
  await bound
  controller.show('p1', 't1')
  const opened = { url: 'about:blank', tab: 't1' }
  expect(controller.startingPhase('p1', opened)).toBeNull()
  expect(controller.busy('p1', opened)).toBe(false)
  // Its Reload is a request of the user's, which loads as any does.
  void controller.history('t1', 'reload')
  expect(controller.busy('p1', opened)).toBe(true)
  end()
})

test('a tab shown again shows no page until the browser’s list names its tab, which a restart may have lost', async () => {
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const { controller, list, end } = harness({ stream: recordingStream(views) }, { pictures: async () => true })
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  const signIn = { url: 'https://github.com/login', tab: 't1', title: 'Sign in to GitHub' }
  controller.show('p1', 't1')
  // The data names a browser tab, but nothing has said yet what the Host holds.
  expect(controller.startingPhase('p1', signIn)).toBe('connecting')
  // The plugin's list lacks it: the browser lost it, and it opens again as any tab does.
  list.value = { tabs: [] }
  expect(controller.startingPhase('p1', signIn)).toBe('browser')
  // The list names it: the content shows the page.
  const row = { id: 't1', title: 'Sign in to GitHub', url: signIn.url, createdBy: { kind: 'user' } as const, loading: false, canGoBack: false, canGoForward: false, shows: 0 }
  list.value = { tabs: [row] }
  expect(controller.startingPhase('p1', signIn)).toBeNull()
  // A view that reports the browser without it outweighs a plugin's list that still names it.
  views[0]!.handlers.data(framed({ type: 'state', running: true, list: 1, tabs: [], watched: null }))
  expect(controller.startingPhase('p1', signIn)).toBe('browser')
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
        return { send: () => {}, close: () => {} }
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

test('a tab not shown that was lost with the browser keeps its title and waits, not loading, until it is shown', async () => {
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const { controller, list, end } = harness({ stream: recordingStream(views) }, { pictures: async () => true })
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('p-t2', 't2')
  const row = (id: string, title: string) => ({
    id, title, url: `https://example.test/${id}`, createdBy: { kind: 'user' } as const,
    loading: false, canGoBack: false, canGoForward: false, shows: 0,
  })
  list.value = { tabs: [row('t1', 'Orders'), row('t2', 'Invoices')] }
  const orders = { url: 'https://example.test/t1', tab: 't1', title: 'Orders' }
  expect(controller.busy('p-t1', orders)).toBe(false)
  // The browser ended: the view and the plugin's list name no tab, before the plugin read the list and marked the panel tabs.
  views[0]!.handlers.data(framed({ type: 'state', running: false, list: 2, tabs: [], watched: null }))
  list.value = { tabs: [] }
  expect(controller.busy('p-t1', orders)).toBe(false)
  expect(browserTabKind.title(orders, { conversation: 'c1', session: controller })).toBe('Orders')
  end()
})

// `plugin-pages.md` § The page context: a call whose caller goes away, as a
// panel that closes, is dropped and never sent.
test('a call still waiting for the backend when the panel session ends is dropped with the abort', async () => {
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

// A failure keeps what it failed for and is worded as it is read, so a
// language chosen while it shows rewords it.
test('a tabs error words its failure in the language the page speaks when it is read', () => {
  const refused = new BrowserTabsError({ status: '500 Internal Server Error' })
  const offline = BrowserTabsError.of('device_offline')
  try {
    expect([refused.message, offline.message]).toEqual(['The server couldn’t do that (500 Internal Server Error).', 'The device is offline.'])
    createPageI18n('ja')
    expect([refused.message, offline.message]).toEqual(['サーバーはそれを実行できませんでした（500 Internal Server Error）。', 'デバイスはオフラインです。'])
  } finally {
    createPageI18n('en')
  }
})

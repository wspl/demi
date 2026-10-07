import { expect, jest, test } from 'bun:test'
import { LIVE_CONTROL_FRAME, LIVE_VIDEO_CODEC, type LiveModuleMessage, type LiveViewerMessage } from '../../generated/plugin'
import type { UserStreamHandlers } from '@demicodes/plugin-sdk'
import { until } from '@vueuse/core'
import { effectScope, ref, shallowRef } from 'vue'
import {
  NO_BROWSER,
  BrowserTabsController,
  BrowserTabsError,
  type BrowserTabList,
  type BrowserTabsApi,
  type BrowserTabsOptions,
  type PictureSupport,
} from '../tabs'

/** A controller in a panel session's effect scope, over a tab list the test sets; what it reports to the user lands in `reported`. */
function harness(api: Partial<BrowserTabsApi>, options: BrowserTabsOptions = {}, reported: Array<[string, unknown]> = []) {
  const list = shallowRef<BrowserTabList | null>(null)
  const error = shallowRef<BrowserTabsError | null>(null)
  let syncs = 0
  const scope = effectScope()
  const controller = scope.run(() => new BrowserTabsController(
    {
      tabs: { value: list, error },
      bind: async () => null,
      sync: async () => void (syncs += 1),
      navigate: async () => 0,
      history: async () => 0,
      stop: async () => 0,
      stream: () => ({ send: () => {}, close: () => {} }),
      installed: () => [],
      ...api,
    },
    { report: (title, error) => void reported.push([title, error]), defect: () => {} },
    { visibility: ref<DocumentVisibilityState>('visible'), ...options },
  ))!
  return { controller, list, syncs: () => syncs, end: () => { controller.dispose(); scope.stop() } }
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

test('a new tab waits for the Host to hold the browser the tab list names', () => {
  const chrome = { name: 'Chrome for Testing', version: '153.0.8010.36' }
  const installed = shallowRef([{ package: 'demi.browser', name: 'program', version: '0.1.3' }])
  const { controller, list, end } = harness({ installed: () => installed.value })
  // Until the first list names the browser, nothing says it is missing.
  expect(controller.unavailable.value).toBeNull()
  list.value = { tabs: [], browser: chrome }
  expect(controller.unavailable.value).toBe(NO_BROWSER)
  // Another version of the line is not the one this Demi pins.
  installed.value = [...installed.value, { package: 'demi.browser', name: chrome.name, version: '152.0.7900.12' }]
  expect(controller.unavailable.value).toBe(NO_BROWSER)
  // The agent's `demi browser install` reports the pinned one.
  installed.value = [...installed.value, { package: 'demi.browser', ...chrome }]
  expect(controller.unavailable.value).toBeNull()
  end()
})

test('a notice the picture shows through is a toast in the page’s words, once; one that leaves no picture is none', async () => {
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const reported: Array<[string, unknown]> = []
  const { controller, end } = harness({ stream: recordingStream(views) }, { pictures: async () => true }, reported)
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('t1')
  views[0]!.handlers.data(framed({ type: 'notice', code: 'input_failed', message: 'Input.dispatchKeyEvent: target closed' }))
  views[0]!.handlers.data(framed({ type: 'notice', code: 'capture_unavailable', message: 'this CPU reports SME without SVE' }))
  expect(reported).toHaveLength(1)
  const [title, error] = reported[0]!
  expect(title).toBe('Could Not Operate the Browser')
  expect(error).toMatchObject({ code: 'input_failed', message: 'The page didn’t receive your input.' })
  expect(controller.session.value?.state.pictureless).toBe('capture_unavailable')
  end()
})

test('a hidden page closes its view, and shown again watches the shown tab on a new view', async () => {
  const visibility = ref<DocumentVisibilityState>('visible')
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean; handlers: UserStreamHandlers }> = []
  const { controller } = harness({ stream: recordingStream(views) }, { visibility, pictures: async () => true })
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  controller.show('t1')
  expect(views).toHaveLength(1)
  // Nobody can watch a hidden page, and an open view would keep its Cloud awake.
  visibility.value = 'hidden'
  expect(views[0]!.closed).toBe(true)
  expect(controller.session.value).toBeNull()
  // Another tab shown meanwhile opens nothing either.
  controller.hide('t1')
  controller.show('t2')
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
  controller.show('t1')
  // The Cloud ran no browser yet, so the view ended; its next try is a wait away.
  opened[0]!.closed('host_stopped')
  expect(opened).toHaveLength(1)
  // The user's new tab got its browser tab: the view does not wait.
  controller.show('t2')
  expect(opened).toHaveLength(2)
  controller.dispose()
})

test('a view that finds its watched tab gone asks the plugin to look, once for that tab', async () => {
  const opened: UserStreamHandlers[] = []
  const { controller, syncs } = harness(
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
  controller.show('t1')
  const state = (tabs: string[]): LiveModuleMessage => ({
    type: 'state',
    running: true,
    list: 1,
    tabs: tabs.map((id) => ({ id, title: id, url: 'about:blank', createdBy: { kind: 'user' }, viewport: VIEWPORT, loading: false, canGoBack: false, canGoForward: false })),
    watched: null,
  })
  opened[0]!.data(framed(state(['t1'])))
  expect(syncs()).toBe(0)
  opened[0]!.data(framed(state([])))
  opened[0]!.data(framed(state([])))
  expect(syncs()).toBe(1)
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
      controller.show('t1')
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
    controller.show('t1')
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
    controller.hide('t1')
    controller.show('t2')
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
  controller.show('t1')
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
  controller.show('t1')
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
  controller.session.value!.showed(1, 0, 0)
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

test('Retry of a tab that could not open loads in the same call, and a failure that comes back ends it', async () => {
  let answer = (_tab: string | null) => {}
  const { controller, end } = harness({ bind: () => new Promise<string | null>((resolve) => (answer = resolve)) })
  const failed = { url: 'https://example.test/', failure: { code: 'device_offline', message: 'Offline' } }
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
  controller.show('t2')
  expect(controller.busy('p1', reopened)).toBe(true)
  end()
})

test('a view that ends asks the plugin whether the browser still has the shown tab', async () => {
  const opened: UserStreamHandlers[] = []
  const { controller, syncs, end } = harness(
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
  controller.show('t1')
  // The Cloud stopped, or the browser ended: the plugin hears of it now, not after the view's waits.
  opened[0]!.closed('host_stopped')
  expect(syncs()).toBe(1)
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
  answers[0]!.reject(new BrowserTabsError('history_boundary', 'The tab has no page to go to in that direction.'))
  expect(await back.catch((error: BrowserTabsError) => error.code)).toBe('history_boundary')
  expect(controller.loading('t1', url)).toBe(false)
  const forward = controller.history('t1', 'forward')
  void controller.history('t1', 'reload')
  answers[1]!.reject(new BrowserTabsError('history_boundary', 'The tab has no page to go to in that direction.'))
  await forward.catch(() => {})
  expect(controller.loading('t1', url)).toBe(true)
  end()
})

/** A view open on the shown `t2`, whose module the test speaks for, and the Stops the plugin was asked for. */
async function openingHarness(bind: () => Promise<string | null>) {
  const opened: UserStreamHandlers[] = []
  const stopped: string[] = []
  const { controller, end } = harness(
    {
      bind,
      stop: async (tab) => {
        stopped.push(tab)
        return 9
      },
      stream: (handlers) => {
        opened.push(handlers)
        return { send: () => {}, close: () => {} }
      },
    },
    { pictures: async () => true },
  )
  await until(controller.pictures).toBe('supported')
  controller.resize(PANEL)
  return { controller, opened, stopped, end }
}

test('a reopened tab loads until its first picture, though the list already says it stopped loading', async () => {
  const { controller, opened, end } = await openingHarness(async () => 't2')
  const reopened = { url: 'https://example.test/', tab: 't2' }
  await controller.bind('p1')
  controller.show('t2')
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
  controller.session.value!.showed(1, 0, 0)
  expect(controller.busy('p1', reopened)).toBe(false)
  end()
})

test('Stop pressed while the tab opens stops its browser tab as soon as it has one', async () => {
  let answer = (_tab: string | null) => {}
  const { controller, stopped, end } = await openingHarness(() => new Promise((resolve) => (answer = resolve)))
  const lost = { url: 'https://example.test/', tab: 't1', closed: true }
  void controller.bind('p1')
  await controller.stopPage('p1', lost)
  // Nothing to stop yet: the lost tab is not the one that opens.
  expect(stopped).toEqual([])
  answer('t2')
  await Promise.resolve()
  await controller.settle('p1', lost)
  expect(stopped).toEqual([])
  await controller.settle('p1', { url: 'https://example.test/', tab: 't2' })
  expect(stopped).toEqual(['t2'])
  // The Stop applied once.
  await controller.settle('p1', { url: 'https://example.test/', tab: 't2' })
  expect(stopped).toEqual(['t2'])
  end()
})

test('Stop pressed on a new tab that then could not open stops nothing', async () => {
  const { controller, stopped, end } = await openingHarness(async () => null)
  await controller.stopPage('p1', { url: 'https://example.test/' })
  await controller.settle('p1', { url: 'https://example.test/', failure: { code: 'device_offline', message: 'Offline' } })
  await controller.settle('p1', { url: 'https://example.test/', tab: 't3' })
  expect(stopped).toEqual([])
  end()
})

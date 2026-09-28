import { expect, test } from 'bun:test'
import { LIVE_VIDEO_CODEC, type LiveViewerMessage } from '@demicodes/protocol'
import { deferred } from '@demicodes/utils'
import { until } from '@vueuse/core'
import { ref } from 'vue'
import {
  BrowserTabsController,
  BrowserTabsError,
  type BrowserTabData,
  type BrowserTabInfo,
  type BrowserTabList,
  type BrowserTabsApi,
  type BrowserTabsOptions,
  type PictureSupport,
} from '../tabs'

const AGENT_TAB: BrowserTabInfo = {
  id: 't1',
  title: 'Login',
  url: 'http://localhost:3000/login',
  createdBy: { kind: 'agent', number: 0 },
}
const USER_TAB: BrowserTabInfo = { id: 't2', title: '', url: 'about:blank', createdBy: { kind: 'user' } }

function harness(api: Partial<BrowserTabsApi>, options: BrowserTabsOptions = {}) {
  const panel: BrowserTabData[] = []
  const controller = new BrowserTabsController(
    {
      list: async () => ({ tabs: [] }),
      open: async () => USER_TAB,
      close: async () => {},
      navigate: async () => {},
      history: async () => {},
      stream: () => ({ send: () => {}, close: () => {} }),
      ...api,
    },
    { bound: () => panel, add: (data) => void panel.push(data) },
    { visibility: ref<DocumentVisibilityState>('visible'), ...options },
  )
  return { controller, panel }
}

/**
 * The browser's `VideoDecoder`, answering by codec, until the returned
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

test('a browser tab no panel tab is bound to is added once, and nothing is ever removed', async () => {
  let tabs = [AGENT_TAB]
  const { controller, panel } = harness({ list: async () => ({ tabs }) })
  await controller.refresh()
  await controller.refresh()
  expect(panel).toEqual([{ url: AGENT_TAB.url, tab: AGENT_TAB.id }])
  // The agent closed it: the panel tab stays, and the list says the browser lost it.
  tabs = []
  await controller.refresh()
  expect(panel).toHaveLength(1)
  expect(controller.list.value).toEqual({ tabs: [] })
})

test('a tab being opened is not taken for the agent\'s, and asking twice opens once', async () => {
  const answer = deferred<BrowserTabInfo>()
  let opens = 0
  const { controller, panel } = harness({
    open: () => {
      opens += 1
      return answer.promise
    },
  })
  const pending: BrowserTabData = { url: 'about:blank' }
  panel.push(pending)
  const bind = (tab: BrowserTabInfo) => {
    panel[0] = { url: tab.url, tab: tab.id }
  }
  const first = controller.open('panel-1', 'about:blank', bind)
  const second = controller.open('panel-1', 'about:blank', bind)
  expect(second).toBe(first)
  // The view already lists the new tab while the request is in flight.
  controller.adopt({ tabs: [USER_TAB] })
  expect(panel).toHaveLength(1)
  answer.resolve(USER_TAB)
  await first
  expect(opens).toBe(1)
  expect(panel).toEqual([{ url: 'about:blank', tab: USER_TAB.id }])
})

test('a refused list keeps the last one and says why', async () => {
  let refuse = false
  const { controller } = harness({
    list: async () => {
      if (refuse) {
        throw new BrowserTabsError('device_offline', 'The device has no live runner')
      }
      return { tabs: [AGENT_TAB] }
    },
  })
  await controller.refresh()
  refuse = true
  await controller.refresh()
  expect(controller.list.value?.tabs).toEqual([AGENT_TAB])
  expect(controller.listError.value?.code).toBe('device_offline')
})

test('a tab the user closed is not taken for the agent\'s while the browser still lists it', async () => {
  const closing = deferred<void>()
  const { controller, panel } = harness({ close: () => closing.promise })
  panel.push({ url: USER_TAB.url, tab: USER_TAB.id })
  // The panel removed its tab and asks the browser to close its own.
  panel.pop()
  const closed = controller.close(USER_TAB.id)
  controller.adopt({ tabs: [USER_TAB] })
  expect(panel).toHaveLength(0)
  closing.resolve()
  await closed
  controller.adopt({ tabs: [] })
  expect(panel).toHaveLength(0)
})

test('a panel tab that lost its binding gets the tab this page opened for it, not a second one', async () => {
  let opens = 0
  const { controller } = harness({
    open: async () => {
      opens += 1
      return USER_TAB
    },
  })
  const bound: string[] = []
  await controller.open('panel-1', 'about:blank', (tab) => void bound.push(tab.id))
  // A stale read of the saved panel took the binding away: the content asks again.
  await controller.open('panel-1', 'about:blank', (tab) => void bound.push(tab.id))
  expect(opens).toBe(1)
  expect(bound).toEqual([USER_TAB.id, USER_TAB.id])
  // The browser lost the tab: asking again opens a new one.
  controller.adopt({ tabs: [] })
  await controller.open('panel-1', 'about:blank', (tab) => void bound.push(tab.id))
  expect(opens).toBe(2)
})

test('a hidden page closes its view, and shown again watches the shown tab on a new view', async () => {
  const visibility = ref<DocumentVisibilityState>('visible')
  const views: Array<{ sent: LiveViewerMessage[]; closed: boolean }> = []
  const decoder = new TextDecoder()
  const { controller } = harness(
    {
      stream: () => {
        const view = { sent: [] as LiveViewerMessage[], closed: false }
        views.push(view)
        return {
          send: (bytes) => void view.sent.push(JSON.parse(decoder.decode(bytes.subarray(5))) as LiveViewerMessage),
          close: () => {
            view.closed = true
          },
        }
      },
    },
    { visibility, pictures: async () => true },
  )
  await until(controller.pictures).toBe('supported')
  controller.show(AGENT_TAB.id)
  expect(views).toHaveLength(1)
  // Nobody can watch a hidden page, and an open view would keep its Cloud awake.
  visibility.value = 'hidden'
  expect(views[0]!.closed).toBe(true)
  expect(controller.session.value).toBeNull()
  // Another tab shown meanwhile opens nothing either.
  controller.hide(AGENT_TAB.id)
  controller.show(USER_TAB.id)
  expect(views).toHaveLength(1)
  visibility.value = 'visible'
  expect(views).toHaveLength(2)
  expect(views[1]!.sent.map((message) => message.type)).toEqual(['hello', 'watch'])
  expect(views[1]!.sent.at(-1)).toEqual({ type: 'watch', tab: USER_TAB.id })
  controller.dispose()
  expect(views[1]!.closed).toBe(true)
})

test('a page shown again reads the tab list and adds the tabs the agent opened while it was hidden', async () => {
  const visibility = ref<DocumentVisibilityState>('visible')
  const answer = deferred<BrowserTabList>()
  let reads = 0
  const { controller, panel } = harness(
    {
      list: () => {
        reads += 1
        return answer.promise
      },
    },
    { visibility },
  )
  visibility.value = 'hidden'
  expect(reads).toBe(0)
  visibility.value = 'visible'
  expect(reads).toBe(1)
  answer.resolve({ tabs: [AGENT_TAB] })
  await answer.promise
  expect(panel).toEqual([{ url: AGENT_TAB.url, tab: AGENT_TAB.id }])
  controller.dispose()
})

test('a browser that cannot decode the pictures opens no view, and one that can opens one', async () => {
  // A Chromium built without proprietary codecs has WebCodecs, and VP8 and VP9, but no H.264.
  // The page asks for the codec the Host's extension encodes, the live protocol's.
  const browsers: Array<[string, ((codec: string) => boolean) | null, PictureSupport, number]> = [
    ['no WebCodecs', null, 'unsupported', 0],
    ['WebCodecs without H.264', (codec) => codec.startsWith('vp'), 'unsupported', 0],
    ['WebCodecs with H.264', (codec) => codec === LIVE_VIDEO_CODEC, 'supported', 1],
  ]
  for (const [browser, decodes, support, viewCount] of browsers) {
    const restore = decodes ? stubDecoder(decodes) : () => {}
    try {
      let views = 0
      const { controller } = harness({
        stream: () => {
          views += 1
          return { send: () => {}, close: () => {} }
        },
      })
      controller.show(AGENT_TAB.id)
      await until(controller.pictures).not.toBe('checking')
      expect({ browser, support: controller.pictures.value, views }).toEqual({ browser, support, views: viewCount })
      controller.dispose()
    } finally {
      restore()
    }
  }
})

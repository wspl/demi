import { expect, test } from 'bun:test'
import { LIVE_CONTROL_FRAME, LIVE_VIDEO_CODEC, type LiveModuleMessage, type LiveViewerMessage } from '../../generated/plugin'
import type { UserStreamHandlers } from '@demicodes/plugin-sdk'
import { until } from '@vueuse/core'
import { effectScope, ref, shallowRef } from 'vue'
import {
  BrowserTabsController,
  type BrowserTabList,
  type BrowserTabsApi,
  type BrowserTabsError,
  type BrowserTabsOptions,
  type PictureSupport,
} from '../tabs'

/** A controller in a panel session's effect scope, over a tab list the test sets. */
function harness(api: Partial<BrowserTabsApi>, options: BrowserTabsOptions = {}) {
  const list = shallowRef<BrowserTabList | null>(null)
  const error = shallowRef<BrowserTabsError | null>(null)
  let syncs = 0
  const scope = effectScope()
  const controller = scope.run(() => new BrowserTabsController(
    {
      tabs: { value: list, error },
      bind: async () => {},
      sync: async () => void (syncs += 1),
      navigate: async () => {},
      history: async () => {},
      stream: () => ({ send: () => {}, close: () => {} }),
      installs: () => [],
      ...api,
    },
    () => {},
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
  expect(views[1]!.sent.map((message) => message.type)).toEqual(['hello', 'watch'])
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
  controller.show('t1')
  const state = (tabs: string[]): LiveModuleMessage => ({
    type: 'state',
    running: true,
    tabs: tabs.map((id) => ({ id, title: id, url: 'about:blank', createdBy: { kind: 'user' }, viewport: VIEWPORT, loading: false })),
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
      controller.show('t1')
      await until(controller.pictures).not.toBe('checking')
      expect({ webBrowser, support: controller.pictures.value, views }).toEqual({ webBrowser, support, views: viewCount })
      controller.dispose()
    } finally {
      restore()
    }
  }
})

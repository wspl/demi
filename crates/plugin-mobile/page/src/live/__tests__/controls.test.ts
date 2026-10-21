import { expect, test } from 'bun:test'
import { createPinia } from 'pinia'
import { computed, effectScope, shallowRef } from 'vue'
import { waitFor } from '@demicodes/utils'
import { pageContext, type PageHost } from '@demicodes/web-ui/plugins/page'
import { disposePluginStores } from '@demicodes/web-ui/plugins/stores'
import { LIVE_CONTROL_FRAME, UNANSWERED_STREAM, definePage, pageReturned, type ConversationPlugin, type PluginCache, type PluginCallError, type UserStreamHandlers } from '@demicodes/plugin-sdk'
import type { LiveDevice, LiveFrame, LiveModuleMessage, LiveViewerMessage, Orientation } from '../../generated/plugin'
import { mobileApi } from '../../devices'
import { MobileDevicesController, noticeText } from '../controller'
import { DEVICE_CONTROLS, deviceControls } from '../controls'
import { DeviceSession } from '../session'
import type { MobileTabData } from '../kind'
import { mobileStore } from '../store'

// The device toolbar's controls and each device's live view on the page's
// side (`crates/plugin-mobile/page/src/index.ts`,
// `docs/decisions/0017-each-device-streams-on-its-own.md`): its own stream,
// whether it is looked at, its look and its frames. The plugin's own tests
// show that each page method runs the operation of its command; here, that
// each control calls the method of the command that does the same. No page,
// a few ms each.

/** What each method answers, as the plugin would for device m1. */
const ANSWERS: Record<string, unknown> = {
  rotate: { device: 'm1', orientation: 'landscape-left', width: 1568, height: 723 },
  toggleAppearance: { device: 'm1', appearance: 'dark' },
  press: { device: 'm1', key: 'home' },
  screenshot: { path: '/scratch/mobile/m1.png' },
  record: { device: 'm1', recording: true },
}

/** The conversation's plugin, keeping each call it gets and each message its streams send. */
function plugin(
  calls: Array<[string, object]>,
  streams: UserStreamHandlers[] = [],
  sent: LiveViewerMessage[] = [],
  answer: (method: string) => Promise<unknown> = async (method) => ANSWERS[method],
  /** What the streams were told of who watches them, and their closes. */
  told: Array<boolean | 'closed'> = [],
): ConversationPlugin {
  const state = shallowRef<unknown>(null)
  return {
    state: (schema) => ({
      value: computed(() => {
        const read = schema.safeParse(state.value)
        return read.success ? read.data : null
      }),
      error: computed<PluginCallError | null>(() => null),
      read: () => {},
    }),
    call: async (method, params, result) => {
      calls.push([method, params])
      return result.parse(await answer(method))
    },
    stream: () => (handlers) => {
      streams.push(handlers)
      return {
        send: (bytes) => {
          sent.push(JSON.parse(new TextDecoder().decode(bytes.subarray(5))) as LiveViewerMessage)
        },
        watched: (watched) => void told.push(watched),
        close: () => void told.push('closed'),
      }
    },
    installed: computed(() => []),
    hostStarting: computed(() => false),
  }
}

test('each toolbar control calls the page method of the command that does the same', async () => {
  const calls: Array<[string, object]> = []
  const reported: unknown[] = []
  const scope = effectScope()
  const controller = scope.run(() => new MobileDevicesController(
    mobileApi(plugin(calls)),
    { report: (_, error) => void reported.push(error), defect: (_, error) => void reported.push(error) },
    { pictures: async () => false, platform: 'mac' },
  ))!
  const saved: Array<string | null> = []
  for (const control of deviceControls('android')) {
    saved.push(await controller.run(control.call('m1', { recording: false })))
  }
  // Record again, now that it records: Stop Recording.
  const record = DEVICE_CONTROLS.at(-1)!
  expect(record.label({ recording: true })).toBe('toolbar.stopRecording')
  await controller.run(record.call('m1', { recording: true }))
  controller.dispose()
  scope.stop()
  // The arguments of `demi mobile rotate m1 left`, `appearance m1 toggle`,
  // `key m1 home`, … as the page method takes them.
  expect(calls).toEqual([
    ['rotate', { device: 'm1', turn: 'left' }],
    ['rotate', { device: 'm1', turn: 'right' }],
    ['toggleAppearance', { device: 'm1' }],
    ['press', { device: 'm1', button: 'home' }],
    ['press', { device: 'm1', button: 'back' }],
    ['press', { device: 'm1', button: 'lock' }],
    ['press', { device: 'm1', button: 'volume-up' }],
    ['press', { device: 'm1', button: 'volume-down' }],
    ['screenshot', { device: 'm1' }],
    ['record', { device: 'm1', action: 'start' }],
    ['record', { device: 'm1', action: 'stop' }],
  ])
  // Screenshot answers the file the content saves on the user's computer.
  expect(saved).toEqual([null, null, null, null, null, null, null, null, '/scratch/mobile/m1.png', null])
  expect(reported).toEqual([])
  // An iPhone has no Back button: its toolbar leaves Back out.
  expect(deviceControls('ios').map((control) => control.label({ recording: false }))).not.toContain('toolbar.back')
})

function framed(message: LiveModuleMessage): Uint8Array {
  const json = new TextEncoder().encode(JSON.stringify(message))
  const bytes = new Uint8Array(5 + json.length)
  new DataView(bytes.buffer).setUint32(0, 1 + json.length)
  bytes[4] = LIVE_CONTROL_FRAME
  bytes.set(json, 5)
  return bytes
}

const IPHONE: LiveDevice = { name: 'iPhone 17', platform: 'ios', os: 'iOS 26.5', model: 'iPhone 17', recording: false, orientation: 'portrait' }
const PIXEL: LiveDevice = { name: 'Pixel 10', platform: 'android', os: 'Android 16', model: 'Pixel 10', recording: false, orientation: 'portrait' }
const ROUND = { kind: 'round' as const, radius: 55 }
const DRAWN = { kind: 'drawn' as const, corners: { topLeft: ROUND, topRight: ROUND, bottomRight: ROUND, bottomLeft: ROUND }, buttons: [] }
const drawn = (radius: number) => ({ ...DRAWN, corners: { topLeft: { kind: 'round' as const, radius }, topRight: { kind: 'round' as const, radius }, bottomRight: { kind: 'round' as const, radius }, bottomLeft: { kind: 'round' as const, radius } } })

/** A disk cache over a map, as the plugin's IndexedDB keeps it. */
function mapCache(kept: Map<string, Uint8Array<ArrayBuffer>>): PluginCache {
  return {
    read: async (key) => kept.get(key) ?? null,
    write: async (key, bytes) => void kept.set(key, bytes),
  }
}

/** Lets the map cache's reads and the frames' decoding under way complete. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 6; turn++) {
    await Promise.resolve()
  }
}

/**
 * Device `device`'s live view on a stream whose Host side is the last of
 * `streams`, its sent messages in `sent` and what its stream was told of
 * who watches it in `told`; enabled, as a browser that shows pictures does.
 */
function deviceView(device = 'm1', options: {
  cache?: PluginCache
  model?: string | null
  gone?: () => void
  decodeImage?: (src: string) => Promise<void>
  defect?: (message: string, error: unknown) => void
  now?: () => number
} = {}) {
  const streams: UserStreamHandlers[] = []
  const sent: LiveViewerMessage[] = []
  const told: Array<boolean | 'closed'> = []
  const session = new DeviceSession({
    open: plugin([], streams, sent, undefined, told).stream('mobile'),
    device,
    model: options.model === undefined ? 'iPhone 17' : options.model,
    nativeId: `native-${device}`,
    cache: options.cache,
    platform: 'mac',
    gone: options.gone,
    defect: options.defect ?? ((_, error) => { throw error }),
    decodeImage: options.decodeImage ?? (async () => {}),
    now: options.now,
  })
  session.enable()
  const host = () => streams.at(-1)!
  return {
    session,
    sent,
    told,
    streams,
    host,
    /** The Host says the device's state, as it does once the stream is looked at. */
    state: (state: LiveDevice = IPHONE) => host().data(framed({ type: 'state', device: state })),
    looks: () => sent.flatMap((message) => message.type === 'looked' ? [message.looked] : []),
    frames: () => sent.filter((message) => message.type === 'frame'),
    /** The Host draws the device's frame for an area of `width` × 1200 device pixels at `scale`. */
    frame: (orientation: Orientation = 'portrait', frame: LiveFrame = DRAWN, width = 800, scale = 2) => {
      const upright = orientation === 'portrait' || orientation === 'portrait-upside-down'
      host().data(framed({
        type: 'frame', width, height: 1200, scale, orientation,
        pointsWidth: upright ? 402 : 874, pointsHeight: upright ? 874 : 402, frame,
      }))
    },
  }
}

test('each device opens a stream of its own, once this browser shows pictures, saying which device it carries', () => {
  const later = deviceView('m1')
  const other = deviceView('m2')
  expect(later.streams).toHaveLength(1)
  expect(other.streams).toHaveLength(1)
  // Its hello names its device; it is not looked at until a view of it shows.
  expect(later.sent).toEqual([{ type: 'hello', platform: 'mac', device: 'm1' }])
  expect(other.sent).toEqual([{ type: 'hello', platform: 'mac', device: 'm2' }])
  expect(later.told).toEqual([false])
  later.session.close()
  other.session.close()

  // A device shown before this web browser has said it shows the pictures opens its stream once it has.
  const streams: UserStreamHandlers[] = []
  const waiting = new DeviceSession({ open: plugin([], streams).stream('mobile'), device: 'm1', model: null, nativeId: null, platform: 'mac', defect: (_, error) => { throw error } })
  waiting.showIn({}, true)
  expect(streams).toHaveLength(0)
  waiting.enable()
  expect(streams).toHaveLength(1)
  waiting.close()
})

test('the view follows the agent’s activity on its device', () => {
  const view = deviceView()
  view.session.showIn({}, true)
  view.state()
  expect(view.session.state.activity).toEqual({ active: false })
  view.host().data(framed({ type: 'activity', activity: { active: true, point: { x: 100, y: 200 } } }))
  expect(view.session.state.activity).toEqual({ active: true, point: { x: 100, y: 200 } })
  view.host().data(framed({ type: 'activity', activity: { active: false, point: { x: 100, y: 200 } } }))
  expect(view.session.state.activity).toEqual({ active: false, point: { x: 100, y: 200 } })
  view.session.close()
})

/**
 * Whether a device's stream is looked at follows its own views and the
 * page alone, in whatever order a tab's show, its hide and the page's
 * visibility come, and each change is said once.
 *
 * Planted defects this catches: a device on screen in a visible page told
 * that nobody looks at it after a hide and a show crossed, which lets it
 * close as idle; a hidden page still looking; a change said twice.
 */
test('a device shown in a visible page is looked at, and one hidden is not, through every order of hides and shows', () => {
  const orders: Array<Array<'show' | 'hide' | 'pageHidden' | 'pageVisible' | 'otherView' | 'otherGone'>> = [
    ['show', 'hide', 'show'],
    ['show', 'pageHidden', 'hide', 'pageVisible', 'show'],
    ['show', 'otherView', 'hide', 'otherGone', 'show'],
    ['pageHidden', 'show', 'pageVisible'],
    ['show', 'pageHidden', 'pageVisible', 'pageHidden', 'pageVisible'],
    ['otherView', 'show', 'otherGone', 'pageHidden', 'pageVisible'],
  ]
  for (const order of orders) {
    const view = deviceView()
    const tab = {}
    const other = {}
    let shown = new Set<object>()
    let visible = true
    for (const step of order) {
      switch (step) {
        case 'show': view.session.showIn(tab, true); shown.add(tab); break
        case 'hide': view.session.showIn(tab, false); shown.delete(tab); break
        case 'otherView': view.session.showIn(other, true); shown.add(other); break
        case 'otherGone': view.session.showIn(other, false); shown.delete(other); break
        case 'pageHidden': view.session.visibility(false); visible = false; break
        case 'pageVisible': view.session.visibility(true); visible = true; break
      }
      const looked = visible && shown.size > 0
      // The Host hears it in the stream, and the backend beside the stream.
      expect([view.looks().at(-1) ?? false, view.told.at(-1)]).toEqual([looked, looked])
    }
    const said = view.looks()
    expect(said.every((value, at) => at === 0 ? value : value !== said[at - 1])).toBe(true)
    view.session.close()
    shown = new Set()
  }
})

/**
 * Two devices looked at together each get their own stream's pictures and
 * state, and one no longer looked at leaves the other looked at.
 *
 * Planted defect this catches: one device's look or Host state reaching
 * another's view, as when the conversation's devices shared a stream.
 */
test('two devices looked at together stream each on its own, and one pausing leaves the other streaming', async () => {
  const iphone = deviceView('m1')
  const pixel = deviceView('m2', { model: 'Pixel 10' })
  for (const view of [iphone, pixel]) {
    view.session.showIn({}, true)
    view.session.frame(800, 1200, 2)
  }
  iphone.state(IPHONE)
  pixel.state(PIXEL)
  iphone.frame('portrait', drawn(55))
  pixel.frame('landscape-left', drawn(30))
  await settle()
  expect(iphone.session.state.device).toEqual(IPHONE)
  expect(pixel.session.state.device).toEqual(PIXEL)
  expect(iphone.session.state.frames.portrait?.frame).toEqual(drawn(55))
  expect(iphone.session.state.frames['landscape-left']).toBeUndefined()
  expect(pixel.session.state.frames['landscape-left']?.frame).toEqual(drawn(30))
  expect(pixel.session.state.frames.portrait).toBeUndefined()

  const tab = {}
  iphone.session.showIn(tab, true)
  pixel.session.visibility(true)
  const pixelLooks = pixel.looks()
  iphone.session.showIn(tab, false)
  iphone.session.visibility(false)
  expect(iphone.looks().at(-1)).toBe(false)
  expect(pixel.looks()).toEqual(pixelLooks)
  expect(pixel.looks().at(-1)).toBe(true)
  iphone.session.close()
  pixel.session.close()
})

test('Record follows a recording the device announces, whoever started it', () => {
  const view = deviceView()
  view.session.showIn({}, true)
  const record = DEVICE_CONTROLS.at(-1)!
  expect(record.call('m1', { recording: view.session.recording }).params).toEqual({ device: 'm1', action: 'start' })
  // The agent, or another window, started a recording.
  view.state({ ...IPHONE, recording: true })
  expect(record.label({ recording: view.session.recording })).toBe('toolbar.stopRecording')
  expect(record.call('m1', { recording: view.session.recording }).params).toEqual({ device: 'm1', action: 'stop' })
  view.state({ ...IPHONE, recording: false })
  expect(record.label({ recording: view.session.recording })).toBe('toolbar.record')
  view.session.close()
})

/**
 * Planted defects these catch: a frame asked for again at a size the Host
 * drew, and one asked for though the disk cache keeps it.
 */
test('the view asks the Host for the frame of its size and pixel ratio once, and only when the disk cache lacks it', async () => {
  const view = deviceView()
  view.session.showIn({}, true)
  view.state()
  // No size yet: nothing to draw for.
  expect(view.frames()).toEqual([])
  view.session.frame(800.4, 1200, 2)
  view.session.frame(800, 1200, 2)
  expect(view.frames()).toEqual([{ type: 'frame', width: 800, height: 1200, scale: 2 }])
  view.session.frame(1000, 1200, 2)
  // The window moved to a screen of another pixel ratio, its size in device pixels the same.
  view.session.frame(1000, 1200, 1)
  expect(view.frames()).toEqual([
    { type: 'frame', width: 800, height: 1200, scale: 2 },
    { type: 'frame', width: 1000, height: 1200, scale: 2 },
    { type: 'frame', width: 1000, height: 1200, scale: 1 },
  ])
  view.session.close()

  const kept = new Map<string, Uint8Array<ArrayBuffer>>()
  const first = deviceView('m1', { cache: mapCache(kept) })
  first.session.showIn({}, true)
  first.state()
  first.session.frame(800, 1200, 2)
  await settle()
  expect(first.frames()).toHaveLength(1)
  first.frame()
  await settle()
  first.session.close()
  const again = deviceView('m1', { cache: mapCache(kept) })
  again.session.showIn({}, true)
  again.state()
  again.session.frame(800, 1200, 2)
  await settle()
  expect(again.frames()).toEqual([])
  expect(again.session.state.frames.portrait?.frame).toEqual(DRAWN)
  again.session.close()
})

test('a device that closes ends its view, which no reconnect opens again', async () => {
  let gone = 0
  const view = deviceView('m1', { gone: () => void (gone += 1) })
  view.session.showIn({}, true)
  view.host().data(framed({ type: 'gone' }))
  expect(gone).toBe(1)
  expect(view.session.state.gone).toBe(true)
  expect(view.told.at(-1)).toBe('closed')
  view.session.visibility(false)
  view.session.visibility(true)
  await pageReturned()
  expect(view.streams).toHaveLength(1)
})

test('text that found no focused field is told in the panel’s own words, quoting the viewer’s text', async () => {
  const streams: UserStreamHandlers[] = []
  const reported: Error[] = []
  const scope = effectScope()
  const controller = scope.run(() => new MobileDevicesController(
    mobileApi(plugin([], streams)),
    { report: (_, error) => void reported.push(error as Error), defect: (_, error) => { throw error } },
    { pictures: async () => true, platform: 'mac' },
  ))!
  await Promise.resolve()
  await Promise.resolve()
  controller.device(TAB).showIn({}, true)
  streams[0]!.data(framed({ type: 'notice', code: 'no_focused_field', message: 'Grüße 東京' }))
  streams[0]!.data(framed({ type: 'notice', code: 'input_failed', message: 'the helper ended' }))
  controller.dispose()
  scope.stop()
  expect(reported.map((error) => error.message)).toEqual([
    noticeText('no_focused_field', 'Grüße 東京'),
    noticeText('input_failed', 'the helper ended'),
  ])
  expect(reported[0]!.message).toContain('Grüße 東京')
  // The Host's own error text never reaches the page.
  expect(reported[1]!.message).not.toContain('the helper ended')
})

test('the view says the size it shows the screen at once for each size', () => {
  const view = deviceView()
  view.session.showIn({}, true)
  const shown = () => view.sent.filter((message) => message.type === 'shown')
  view.session.showsScreenAt(860.2, 1868)
  view.session.showsScreenAt(860, 1868)
  view.session.showsScreenAt(600, 1304)
  expect(shown()).toEqual([
    { type: 'shown', width: 860, height: 1868 },
    { type: 'shown', width: 600, height: 1304 },
  ])
  view.session.close()
})

test('the view follows whether the device’s screen is off, and Wake asks it to turn on', () => {
  const view = deviceView()
  view.session.showIn({}, true)
  view.host().data(framed({ type: 'screen', on: false }))
  expect(view.session.state.screenOff).toBe(true)
  view.host().data(framed({ type: 'screen', on: true }))
  expect(view.session.state.screenOff).toBe(false)
  view.session.wake()
  expect(view.sent.filter((message) => message.type === 'wake')).toEqual([{ type: 'wake' }])
  view.session.close()
})

const TAB: MobileTabData = { device: 'm1', nativeId: 'native-m1', name: 'Pixel 10', platform: 'android', model: 'Pixel 10' }

/**
 * A controller over a device m1 whose live view is open and looked at, its
 * frame ready, and whose rotations wait until the test answers each, in
 * turn: `answer` gives the next one the device's answer and waits for its
 * press to end, `refuse` its refusal. `reports` sends the device's notice
 * that its pictures now come in an orientation, as the module announces
 * each generation, whoever turned it.
 */
async function rotating() {
  const waiting: Array<{ resolve: (value: unknown) => void; reject: (error: Error) => void }> = []
  const streams: UserStreamHandlers[] = []
  const reported: unknown[] = []
  const scope = effectScope()
  const controller = scope.run(() => new MobileDevicesController(
    mobileApi(plugin([], streams, [], (method) => new Promise((resolve, reject) => {
      if (method !== 'rotate') {
        resolve(ANSWERS[method])
        return
      }
      waiting.push({ resolve, reject })
    }))),
    { report: (_, error) => void reported.push(error), defect: (_, error) => { throw error } },
    { pictures: async () => true, platform: 'mac' },
  ))!
  await waitFor(() => controller.pictures.value === 'supported')
  const session = controller.device(TAB)
  session.showIn({}, true)
  session.frame(800, 1200, 2)
  const presses: Array<Promise<unknown>> = []
  let generation = 0
  const open = () => {
    streams.at(-1)!.data(framed({ type: 'state', device: PIXEL }))
    streams.at(-1)!.data(framed({ type: 'frame', width: 800, height: 1200, scale: 2, orientation: 'portrait', pointsWidth: 412, pointsHeight: 924, frame: DRAWN }))
  }
  open()
  await settle()
  const reports = (orientation: Orientation) => {
    generation += 1
    const upright = orientation === 'portrait' || orientation === 'portrait-upside-down'
    streams.at(-1)!.data(framed({
      type: 'stream', generation,
      width: upright ? 1080 : 2424, height: upright ? 2424 : 1080,
      pointsWidth: upright ? 412 : 924, pointsHeight: upright ? 924 : 412,
      orientation, scale: 1,
    }))
  }
  return {
    session,
    reported,
    reports,
    streams,
    /** The connection drops and opens again: its generations start over, and nothing it reported stays. */
    reconnect: async () => {
      const before = streams.length
      streams.at(-1)!.closed('the connection dropped')
      generation = 0
      // The page's return opens it again without the rest of its wait.
      await pageReturned()
      expect(streams).toHaveLength(before + 1)
    },
    rotate: (turn: 'left' | 'right') => {
      presses.push(controller.run({ method: 'rotate', params: { device: 'm1', turn } }))
    },
    /** The quarter turns the view adds to what it draws, of `drawn`. */
    quarters: (drawn: Orientation) => session.pendingQuarters(drawn),
    answer: async (orientation: Orientation) => {
      waiting.shift()!.resolve({ device: 'm1', orientation, width: 1, height: 1 })
      await presses.shift()
    },
    refuse: async () => {
      waiting.shift()!.reject(new Error('the device did not turn'))
      await presses.shift()
    },
    end: () => {
      controller.dispose()
      scope.stop()
    },
  }
}

test('a rotation turns what the view shows at once until a picture in the device’s new orientation is drawn, however late it answers', async () => {
  const device = await rotating()
  device.reports('portrait')
  // Rotate Right: a quarter clockwise before the device has answered.
  device.rotate('right')
  expect(device.quarters('portrait')).toBe(1)
  // The device announces its landscape pictures; the portrait one stays drawn, turned.
  device.reports('landscape-right')
  expect(device.quarters('portrait')).toBe(1)
  expect(device.quarters('landscape-right')).toBe(0)
  // Its answer comes after the pictures: nothing turns again.
  await device.answer('landscape-right')
  expect(device.quarters('landscape-right')).toBe(0)
  expect(device.session.state.turning).toEqual([])
  expect(device.reported).toEqual([])
  device.end()
})

test('a refused rotation turns the picture back and shows the error', async () => {
  const device = await rotating()
  device.reports('portrait')
  device.rotate('left')
  expect(device.quarters('portrait')).toBe(-1)
  await device.refuse()
  expect(device.quarters('portrait')).toBe(0)
  expect(device.session.state.turning).toEqual([])
  expect(device.reported).toHaveLength(1)
  device.end()
})

test('quick presses add up, a refused one among them comes off, and the device’s pictures end them', async () => {
  const device = await rotating()
  device.reports('portrait')
  device.rotate('right')
  device.rotate('right')
  device.rotate('right')
  // Three quarters clockwise show as one counterclockwise.
  expect(device.quarters('portrait')).toBe(-1)
  await device.answer('landscape-right')
  await device.refuse()
  await device.answer('portrait-upside-down')
  // Two of the three turned the device: upside down.
  expect(device.quarters('portrait')).toBe(2)
  expect(device.reported).toHaveLength(1)
  // The device's pictures came by its last orientation only: every press is done.
  device.reports('portrait-upside-down')
  expect(device.quarters('portrait-upside-down')).toBe(0)
  expect(device.session.state.turning).toEqual([])
  device.end()
})

test('the viewer and the agent turning the device together end where the device does', async () => {
  const device = await rotating()
  device.reports('portrait')
  device.rotate('right')
  // The agent's turn right and then the viewer's come before the viewer's answer.
  device.reports('landscape-right')
  device.reports('portrait-upside-down')
  expect(device.quarters('portrait-upside-down')).toBe(0)
  await device.answer('portrait-upside-down')
  expect(device.quarters('portrait-upside-down')).toBe(0)
  expect(device.session.state.turning).toEqual([])
  // Again, the device announcing only where both turns ended.
  device.rotate('right')
  device.reports('landscape-left')
  await device.answer('landscape-left')
  expect(device.quarters('landscape-left')).toBe(0)
  expect(device.session.state.turning).toEqual([])
  device.end()
})

test('a refused turn while the agent turned the device shows the device’s orientation', async () => {
  const device = await rotating()
  device.reports('portrait')
  device.rotate('left')
  // The agent turns it right meanwhile.
  device.reports('landscape-right')
  await device.refuse()
  expect(device.quarters('landscape-right')).toBe(0)
  expect(device.quarters('portrait')).toBe(1)
  expect(device.session.state.turning).toEqual([])
  device.end()
})

test('a turn the device answered before a reconnect ends with the new connection’s pictures', async () => {
  const device = await rotating()
  for (const orientation of ['portrait', 'landscape-right', 'portrait-upside-down', 'landscape-left', 'portrait'] as const) {
    device.reports(orientation)
  }
  device.rotate('right')
  await device.answer('landscape-right')
  await device.reconnect()
  // The new connection's generations start at 1.
  device.reports('landscape-right')
  expect(device.quarters('landscape-right')).toBe(0)
  expect(device.session.state.turning).toEqual([])
  device.end()
})

/**
 * After a reconnect, before the new connection reports any orientation,
 * a press still turns what the view shows at once, from the orientation it
 * shows. Planted defect this catches: a rotation that turns only from the
 * orientation the current connection's pictures reported, which a
 * reconnect clears, so the press does nothing until the device answers.
 */
test('a rotation after a reconnect turns what the view shows at once', async () => {
  const device = await rotating()
  device.reports('portrait')
  await device.reconnect()
  expect(device.session.state.reported).toBeNull()
  device.rotate('right')
  expect(device.quarters('portrait')).toBe(1)
  device.reports('landscape-right')
  await device.answer('landscape-right')
  expect(device.quarters('landscape-right')).toBe(0)
  device.end()
})

/** A page host over `conversation`, the conversation plugin every conversation of the test has, with a Pinia of its own. */
function storeHost(conversation: ConversationPlugin): PageHost {
  const unused = () => {
    throw new Error('the test does not use this part of the host')
  }
  return {
    userState: () => undefined,
    followState: () => ({ value: () => null, error: () => null, read: () => {}, stop: () => {} }),
    call: async (_plugin, method, params) => conversation.call(method, params, { parse: (value: unknown) => value } as never),
    stream: () => conversation.stream('mobile'),
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
 * Planted defects this catches: a conversation's simulators that end with
 * its panel, which drop each device's look and open new streams on return,
 * and a device left looked at with no view of it shown, which keeps the
 * conversation awake.
 */
test('a return to a conversation shows its device’s look at once, on the stream it kept', async () => {
  const streams: UserStreamHandlers[] = []
  const sent: LiveViewerMessage[] = []
  const told: Array<boolean | 'closed'> = []
  const host = storeHost(plugin([], streams, sent, async (method) => ANSWERS[method], told))
  const page = pageContext(host, definePage({ plugin: 'mobile' }))
  const devices = page.store(mobileStore({ pictures: async () => true, platform: 'mac' }))

  // The panel shows the device's tab: its content watches the conversation's simulators.
  const shown = devices.watch('c1')
  await waitFor(() => shown.session.devices.pictures.value === 'supported')
  const session = shown.session.devices.device(TAB)
  const tab = {}
  session.showIn(tab, true)
  session.frame(800, 1200, 2)
  streams[0]!.data(framed({ type: 'state', device: PIXEL }))
  streams[0]!.data(framed({ type: 'frame', width: 800, height: 1200, scale: 2, orientation: 'portrait', pointsWidth: 412, pointsHeight: 924, frame: DRAWN }))
  await waitFor(() => session.state.look !== null)
  const look = session.state.look
  // The user switches to another conversation: the content unmounts.
  session.showIn(tab, false)
  shown.end()
  expect(told).toEqual([false, true, false])
  devices.watch('c2').end()

  // And back: the same device's view, its look at once, and the same stream looked at again.
  const back = devices.watch('c1')
  expect(back.session.devices.device(TAB)).toBe(session)
  expect(session.state.look).toEqual(look)
  session.showIn(tab, true)
  expect(streams).toHaveLength(1)
  expect(told).toEqual([false, true, false, true])
  back.end()
  // Turned off, the plugin's store ends the stream.
  disposePluginStores(host.pinia, 'mobile')
  expect(told.at(-1)).toBe('closed')
})

/**
 * Planted defects this catches: a frame cached under a key without the size,
 * the model or the orientation, which would show one device's frame drawn for
 * another size or model, and none read back at all.
 */
test('a device shows the frame the disk cache keeps for its model, size, pixel ratio and orientation, after a reload', async () => {
  const kept = new Map<string, Uint8Array<ArrayBuffer>>()
  const opened = (model = 'iPhone 17', size = 800, scale = 2) => {
    const view = deviceView('m1', { cache: mapCache(kept), model })
    view.session.showIn({}, true)
    view.session.frame(size, 1200, scale)
    return view
  }
  const first = opened()
  first.state()
  first.frame('portrait', DRAWN)
  await waitFor(() => kept.size === 2)
  first.session.close()
  const same = opened()
  await waitFor(() => same.session.state.frames.portrait !== undefined)
  expect(same.session.state.frames.portrait?.frame).toEqual(DRAWN)
  same.session.close()

  // Another size, pixel ratio or model waits for the Host's.
  for (const other of [opened('iPhone 17', 1000), opened('iPhone 17', 800, 1), opened('iPhone 17 Pro')]) {
    await settle()
    expect(other.session.state.frames.portrait).toBeUndefined()
    other.session.close()
  }
})

/**
 * A reloaded page shows the device in its cached frame, in the orientation
 * the cache kept, before the Host has said anything, and asks the Host for
 * no frame. Planted defects this catch: a cached frame that waits for the
 * Host's state or pictures before it shows, which leaves the view
 * connecting for a round trip and more.
 */
test('a cached device shows in its frame before the Host answers', async () => {
  const kept = new Map<string, Uint8Array<ArrayBuffer>>()
  const first = deviceView('m1', { cache: mapCache(kept) })
  first.session.showIn({}, true)
  first.session.frame(800, 1200, 2)
  first.state({ ...IPHONE, orientation: 'landscape-left' })
  first.frame('landscape-left', DRAWN)
  await waitFor(() => kept.size === 2)
  first.session.close()

  const reloaded = deviceView('m1', { cache: mapCache(kept) })
  reloaded.session.showIn({}, true)
  reloaded.session.frame(800, 1200, 2)
  await waitFor(() => reloaded.session.state.look !== null)
  expect(reloaded.session.state.look).toEqual({ orientation: 'landscape-left', pointsWidth: 874, pointsHeight: 402, frame: DRAWN, picture: null })
  expect(reloaded.session.waiting).toBe('connecting')
  expect(reloaded.frames()).toEqual([])
  reloaded.session.close()
})

/**
 * Planted defects this catches: a frame kept under the size the view has when
 * it arrives rather than the size its request named, which a later view of
 * the new size draws at the wrong size, and a frame read for a size the view
 * left before the read completed, which it then shows.
 */
test('a panel resized while a frame is under way keeps and shows each frame only for the size it was drawn for', async () => {
  const kept = new Map<string, Uint8Array<ArrayBuffer>>()
  const resized = deviceView('m1', { cache: mapCache(kept) })
  resized.session.showIn({}, true)
  resized.state()
  resized.session.frame(800, 1200, 2)
  resized.session.frame(1000, 1200, 2)
  resized.frame('portrait', DRAWN, 800)
  await waitFor(() => kept.has('frame:iPhone 17:800x1200@2:portrait'))
  expect(kept.has('frame:iPhone 17:1000x1200@2:portrait')).toBe(false)
  resized.session.close()
})

/** An installed frame's image, its screen upright or across. */
const image = (across = false): LiveFrame => ({
  kind: 'image', png: 'UE5H', width: across ? 800 : 400, height: across ? 400 : 800,
  screen: across ? { x: 30, y: 20, width: 740, height: 360 } : { x: 20, y: 30, width: 360, height: 740 },
  silhouette: null,
})

/**
 * A frame shows only for the orientation it was drawn for: before any
 * picture, the device shows in the frame of the orientation it has, and a
 * frame the Host drew for a screen that turned since is kept for its own
 * orientation while the view asks again; a frame whose screen has the other
 * orientation's shape never shows, nor is kept.
 *
 * Planted defects this catches: the view asking once per size whatever came
 * back, which left it on its frame's wait for good; a frame kept and shown
 * under an orientation whose shape its screen does not have.
 */
test('a frame shows only for the orientation it was drawn for, and the view asks again for the device’s own', async () => {
  const kept = new Map<string, Uint8Array<ArrayBuffer>>()
  const view = deviceView('m1', { cache: mapCache(kept) })
  view.session.showIn({}, true)
  view.session.frame(800, 1200, 2)
  view.state({ ...IPHONE, orientation: 'portrait' })
  await settle()
  expect(view.frames()).toHaveLength(1)
  // The Host drew the frame for a screen turned before it said so.
  view.frame('landscape-left', DRAWN)
  await settle()
  expect(view.session.state.look).toBeNull()
  expect(view.session.state.frames['landscape-left']?.frame).toEqual(DRAWN)
  expect(view.frames()).toHaveLength(2)
  view.frame('portrait', DRAWN)
  await settle()
  expect(view.session.state.look?.orientation).toBe('portrait')
  view.session.close()

  // An installed frame named portrait whose screen lies across.
  const reported: unknown[] = []
  const wrong = deviceView('m2', { cache: mapCache(kept), model: 'iPhone Air', defect: (_, error) => void reported.push(error) })
  wrong.session.showIn({}, true)
  wrong.session.frame(800, 1200, 2)
  wrong.state({ ...IPHONE, model: 'iPhone Air' })
  await settle()
  wrong.frame('portrait', image(true))
  await settle()
  expect(wrong.session.state.frames.portrait).toBeUndefined()
  expect(wrong.session.state.look).toBeNull()
  expect(kept.has('frame:iPhone Air:800x1200@2:portrait')).toBe(false)
  expect(reported).toHaveLength(1)
  wrong.session.close()
})

/**
 * A frame whose image does not decode is kept nowhere, and the view says
 * it could not show it rather than wait for it; Retry asks the Host for it
 * again, and the frame it then sends shows.
 *
 * Planted defects this catches: a frame cached before it decoded, which
 * every reload then failed on, and a failure that left the view loading
 * the frame for good.
 */
test('a frame that cannot be shown says so, is not kept, and Retry asks for it again', async () => {
  const kept = new Map<string, Uint8Array<ArrayBuffer>>()
  let decodes = false
  const reported: unknown[] = []
  const view = deviceView('m1', {
    cache: mapCache(kept),
    decodeImage: async () => {
      if (!decodes) {
        throw new Error('the image does not decode')
      }
    },
    defect: (_, error) => void reported.push(error),
  })
  view.session.showIn({}, true)
  view.session.frame(800, 1200, 2)
  view.state()
  await settle()
  view.frame('portrait', image())
  await settle()
  expect(view.session.waiting).toBe('frameFailed')
  expect([...kept.keys()].some((key) => key.startsWith('frame:'))).toBe(false)
  expect(view.frames()).toHaveLength(1)
  decodes = true
  view.session.retryFrame()
  expect(view.session.waiting).not.toBe('frameFailed')
  expect(view.frames()).toHaveLength(2)
  view.frame('portrait', image())
  await settle()
  expect(view.session.state.look?.frame).toEqual(image())
  expect(kept.has('frame:iPhone 17:800x1200@2:portrait')).toBe(true)
  expect(reported).toHaveLength(1)
  view.session.close()
})

/**
 * A view shown again expects its device's state from the Host, which
 * answers its look: a stream that does not answer within the time a page
 * socket's pong may take, as one whose connection was lost silently while
 * paused, opens again, and one that answers stays.
 *
 * Planted defect this catches: a paused stream's lost connection found
 * only after the page's silence limit, a minute and more after the view
 * showed again.
 */
test('a view shown again opens its stream anew when the Host does not answer in time', () => {
  let now = 0
  const view = deviceView('m1', { now: () => now })
  const tab = {}
  view.session.showIn(tab, true)
  view.state()
  view.session.showIn(tab, false)
  now += 120_000
  view.session.tick()
  expect(view.streams).toHaveLength(1)
  // Shown again: the Host answers in time.
  view.session.showIn(tab, true)
  now += 1_000
  view.state()
  now += 10_000
  view.session.tick()
  expect(view.session.state.ended).toBeNull()
  view.session.showIn(tab, false)
  // Shown again: nothing answers.
  view.session.showIn(tab, true)
  now += 4_000
  view.session.tick()
  expect(view.session.state.ended).toBeNull()
  now += 2_000
  view.session.tick()
  expect(view.session.state.ended).toBe(UNANSWERED_STREAM)
  view.session.close()
})

/**
 * A press the device answers with the orientation it shows, as when the
 * app in front refuses the turn, ends at once: nothing turns, and the view
 * shows the device as it is. Planted defect this catches: such a press
 * kept until pictures of the asked orientation came, which never did, so
 * the view stayed turned a quarter from the device.
 */
test('a rotation the device answers with the orientation it shows turns nothing', async () => {
  const device = await rotating()
  device.reports('portrait')
  device.rotate('right')
  expect(device.quarters('portrait')).toBe(1)
  await device.answer('portrait')
  expect(device.quarters('portrait')).toBe(0)
  expect(device.session.state.turning).toEqual([])
  device.end()
})

import { expect, test } from 'bun:test'
import { computed, effectScope, shallowRef } from 'vue'
import { waitFor } from '@demicodes/utils'
import { LIVE_CONTROL_FRAME, UNANSWERED_STREAM, pageReturned, type ConversationPlugin, type PluginCache, type PluginCallError, type UserStreamHandlers } from '@demicodes/plugin-sdk'
import type { LiveDevice, LiveFrame, LiveModuleMessage, LiveViewerMessage, Orientation } from '../../generated/plugin'
import { mobileApi } from '../../devices'
import { MobileDevicesController } from '../controller'
import { DeviceSession } from '../session'
import type { MobileTabData } from '../kind'

// Each device's live view on the page's side
// (`crates/plugin-mobile/page/src/index.ts`,
// `docs/decisions/0017-each-device-streams-on-its-own.md`): whether it is
// looked at, how a turn settles and its frames. No page, a few ms each.

/** What each method answers, as the plugin would for device m1. */
const ANSWERS: Record<string, unknown> = {
  rotate: { device: 'm1', orientation: 'landscape-left', width: 1568, height: 723 },
  toggleAppearance: { device: 'm1', appearance: 'dark' },
  press: { device: 'm1', key: 'home' },
  screenshot: { path: '/scratch/mobile/m1.png' },
  record: { recording: { id: 'r1', target: 'm1', running: true, durationMs: 0, frames: 0, missed: 0 } },
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

import { expect, test } from 'bun:test'
import { createPinia } from 'pinia'
import { computed, effectScope, shallowRef } from 'vue'
import { waitFor } from '@demicodes/utils'
import { pageContext, type PageHost } from '@demicodes/web-ui/plugins/page'
import { disposePluginStores } from '@demicodes/web-ui/plugins/stores'
import { LIVE_CONTROL_FRAME, definePage, pageReturned, type ConversationPlugin, type PluginCache, type PluginCallError, type UserStreamHandlers } from '@demicodes/plugin-sdk'
import type { LiveDevice, LiveModuleMessage, LiveViewerMessage, Orientation } from '../../generated/plugin'
import { mobileApi } from '../../devices'
import { MobileDevicesController, noticeText } from '../controller'
import { DEVICE_CONTROLS, deviceControls } from '../controls'
import { MobileSession, type DeviceStream } from '../session'
import { mobileStore } from '../store'

// The device toolbar's controls and the agent's activity on the page's side
// (`crates/plugin-mobile/page/src/index.ts`). The plugin's own tests show that each page
// method runs the operation of its command; here, that each control calls
// the method of the command that does the same. No page, a few ms.

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

test('the view follows the agent’s activity on the device it watches, and on no other', () => {
  const streams: UserStreamHandlers[] = []
  const defects: unknown[] = []
  const session = new MobileSession({
    open: plugin([], streams).stream('mobile'),
    platform: 'mac',
    defect: (_, error) => void defects.push(error),
  })
  session.start()
  const host = streams[0]!
  const devices = [
    { id: 'm1', name: 'iPhone 17', platform: 'ios' as const, os: 'iOS 26.5', model: 'iPhone 17', recording: false, orientation: 'portrait' as const },
    { id: 'm2', name: 'iPad Air', platform: 'ios' as const, os: 'iOS 26.5', model: 'iPad Air 11-inch (M3)', recording: false, orientation: 'portrait' as const },
  ]
  host.data(framed({ type: 'state', devices, watched: 'm1' }))
  expect(session.state.activity).toEqual({ active: false })

  host.data(framed({ type: 'activity', device: 'm1', activity: { active: true, point: { x: 100, y: 200 } } }))
  expect(session.state.activity).toEqual({ active: true, point: { x: 100, y: 200 } })
  // The agent's input on another device is not this view's.
  host.data(framed({ type: 'activity', device: 'm2', activity: { active: false } }))
  expect(session.state.activity.active).toBe(true)
  host.data(framed({ type: 'activity', device: 'm1', activity: { active: false, point: { x: 100, y: 200 } } }))
  expect(session.state.activity).toEqual({ active: false, point: { x: 100, y: 200 } })

  // Watching another device starts with no activity, until the Host reports it.
  host.data(framed({ type: 'activity', device: 'm1', activity: { active: true } }))
  session.showing('m2')
  expect(session.state.activity).toEqual({ active: false })
  session.close()
  expect(defects).toEqual([])
})

test('the view opens no stream until a device tab is shown, and opens it for the first one', async () => {
  // A panel with no device tab shown opens none, once this web browser says it shows the pictures.
  const panelStreams: UserStreamHandlers[] = []
  const scope = effectScope()
  const controller = scope.run(() => new MobileDevicesController(
    mobileApi(plugin([], panelStreams)),
    { report: (_, error) => { throw error }, defect: (_, error) => { throw error } },
    { pictures: async () => true, platform: 'mac' },
  ))!
  await Promise.resolve()
  await Promise.resolve()
  expect(panelStreams).toHaveLength(0)
  controller.session.showing('m1')
  expect(panelStreams).toHaveLength(1)
  controller.dispose()
  scope.stop()

  const streams: UserStreamHandlers[] = []
  const session = new MobileSession({
    open: plugin([], streams).stream('mobile'),
    platform: 'mac',
    defect: (_, error) => { throw error },
  })
  session.enable()
  expect(streams).toHaveLength(0)
  session.showing('m1')
  expect(streams).toHaveLength(1)
  session.showing('m2')
  expect(streams).toHaveLength(1)
  session.close()

  // A device tab shown before this web browser has said it shows the pictures opens the stream once it has.
  const later: UserStreamHandlers[] = []
  const waiting = new MobileSession({ open: plugin([], later).stream('mobile'), platform: 'mac', defect: (_, error) => { throw error } })
  waiting.showing('m1')
  expect(later).toHaveLength(0)
  waiting.enable()
  expect(later).toHaveLength(1)
  waiting.close()
})

/** A started session on a stream whose sent messages `sent` keeps, its Host's side the first of `streams`. */
function viewing(sent: LiveViewerMessage[], streams: UserStreamHandlers[] = []): MobileSession {
  const session = new MobileSession({
    open: plugin([], streams, sent).stream('mobile'),
    platform: 'mac',
    defect: (_, error) => { throw error },
  })
  session.enable()
  return session
}

const watches = (sent: LiveViewerMessage[]) => sent.flatMap((message) => message.type === 'watch' ? [message.device] : [])

test('the view lets a device go once none of its tabs is shown or the page is hidden, and never in a switch', async () => {
  const sent: LiveViewerMessage[] = []
  const session = viewing(sent)
  session.showing('m1')
  // A switch to m2's tab, its show before m1's hide, and back, its hide first.
  session.showing('m2')
  session.hiding('m1')
  await Promise.resolve()
  session.hiding('m2')
  session.showing('m1')
  await Promise.resolve()
  expect(watches(sent)).toEqual(['m1', 'm2', 'm1'])

  // The last shown tab is hidden, as when the panel shows a file: nobody watches,
  // so the device gets its hour.
  session.hiding('m1')
  await Promise.resolve()
  expect(watches(sent)).toEqual(['m1', 'm2', 'm1', null])
  session.showing('m1')
  session.visibility(false)
  session.visibility(true)
  expect(watches(sent)).toEqual(['m1', 'm2', 'm1', null, 'm1', null, 'm1'])
  session.close()
})

test('Record stops a recording the device announces, whoever started it', () => {
  const streams: UserStreamHandlers[] = []
  const session = viewing([], streams)
  session.showing('m1')
  const record = DEVICE_CONTROLS.at(-1)!
  const recordingOf = (recording: boolean) => framed({
    type: 'state',
    devices: [{ id: 'm1', name: 'iPhone 17', platform: 'ios', os: 'iOS 26.5', model: 'iPhone 17', recording, orientation: 'portrait' as const }],
    watched: 'm1',
  })
  expect(record.call('m1', { recording: session.recording('m1') }).params).toEqual({ device: 'm1', action: 'start' })
  // The agent, or another window, started a recording.
  streams[0]!.data(recordingOf(true))
  expect(record.label({ recording: session.recording('m1') })).toBe('toolbar.stopRecording')
  expect(record.call('m1', { recording: session.recording('m1') }).params).toEqual({ device: 'm1', action: 'stop' })
  streams[0]!.data(recordingOf(false))
  expect(record.label({ recording: session.recording('m1') })).toBe('toolbar.record')
  session.close()
})

test('the view asks for the frame of its laid-out size and pixel ratio once, again for a new size, ratio or device, and keeps the watched device’s', () => {
  const streams: UserStreamHandlers[] = []
  const sent: LiveViewerMessage[] = []
  const session = viewing(sent, streams)
  const frames = () => sent.filter((message) => message.type === 'frame')
  // No size yet: nothing to draw for.
  session.showing('m1')
  expect(frames()).toEqual([])
  session.frame(800.4, 1200, 2)
  session.frame(800, 1200, 2)
  expect(frames()).toEqual([{ type: 'frame', device: 'm1', width: 800, height: 1200, scale: 2 }])
  session.frame(1000, 1200, 2)
  // The window moved to a screen of another pixel ratio, its size in device pixels the same.
  session.frame(1000, 1200, 1)
  session.showing('m2')
  expect(frames()).toEqual([
    { type: 'frame', device: 'm1', width: 800, height: 1200, scale: 2 },
    { type: 'frame', device: 'm1', width: 1000, height: 1200, scale: 2 },
    { type: 'frame', device: 'm1', width: 1000, height: 1200, scale: 1 },
    { type: 'frame', device: 'm2', width: 1000, height: 1200, scale: 1 },
  ])
  const round = { kind: 'round' as const, radius: 55 }
  const drawnFrame = { kind: 'drawn' as const, corners: { topLeft: round, topRight: round, bottomRight: round, bottomLeft: round }, buttons: [] }
  streams[0]!.data(framed({ type: 'frame', device: 'm1', width: 800, height: 1200, scale: 2, orientation: 'portrait', pointsWidth: 402, pointsHeight: 874, frame: drawnFrame }))
  expect(session.state.frames).toEqual({})
  streams[0]!.data(framed({ type: 'frame', device: 'm2', width: 1000, height: 1200, scale: 1, orientation: 'landscape-left', pointsWidth: 874, pointsHeight: 402, frame: drawnFrame }))
  expect(session.state.frames).toEqual({ m2: { device: 'm2', orientation: 'landscape-left', pointsWidth: 874, pointsHeight: 402, frame: drawnFrame } })
  session.close()
})

test('each device keeps its last frame across a switch of tabs until a newer one comes, and loses it when it closes', async () => {
  // Over a far Host the frame asked for again on a return comes a round
  // trip later: meanwhile the device shows the frame it had.
  const streams: UserStreamHandlers[] = []
  const sent: LiveViewerMessage[] = []
  const session = viewing(sent, streams)
  const round = (radius: number) => ({ kind: 'round' as const, radius })
  const drawn = (radius: number) => ({
    kind: 'drawn' as const,
    corners: { topLeft: round(radius), topRight: round(radius), bottomRight: round(radius), bottomLeft: round(radius) },
    buttons: [],
  })
  const frameOf = (device: string, radius: number) => framed({ type: 'frame', device, width: 800, height: 1200, scale: 2, orientation: 'portrait', pointsWidth: 402, pointsHeight: 874, frame: drawn(radius) })
  const devices = [
    { id: 'm1', name: 'iPhone 17', platform: 'ios' as const, os: 'iOS 26.5', model: 'iPhone 17', recording: false, orientation: 'portrait' as const },
    { id: 'm2', name: 'Pixel 10', platform: 'android' as const, os: 'Android 16', model: 'Pixel 10', recording: false, orientation: 'portrait' as const },
  ]
  session.showing('m1')
  session.frame(800, 1200, 2)
  streams[0]!.data(framed({ type: 'state', devices, watched: 'm1' }))
  streams[0]!.data(frameOf('m1', 55))
  // Another tab, then m1's again.
  session.hiding('m1')
  await Promise.resolve()
  expect(session.state.watched).toBeNull()
  expect(session.state.frames.m1?.frame).toEqual(drawn(55))
  const asked = sent.filter((message) => message.type === 'frame').length
  session.showing('m1')
  expect(session.state.frames.m1?.frame).toEqual(drawn(55))
  // The frame is asked for again, and the newer one replaces the kept one.
  expect(sent.filter((message) => message.type === 'frame')).toHaveLength(asked + 1)
  streams[0]!.data(frameOf('m1', 60))
  expect(session.state.frames.m1?.frame).toEqual(drawn(60))
  // m1 closes: its frame goes with it.
  streams[0]!.data(framed({ type: 'state', devices: [devices[1]!], watched: null }))
  expect(session.state.frames).toEqual({})
  session.close()
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
  controller.session.showing('m1')
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

test('the view says the size it shows the screen at once for each size, and again for the next device it watches', () => {
  const sent: LiveViewerMessage[] = []
  const session = viewing(sent)
  const shown = () => sent.filter((message) => message.type === 'shown')
  session.showing('m1')
  session.showsScreenAt(860.2, 1868)
  session.showsScreenAt(860, 1868)
  session.showsScreenAt(600, 1304)
  session.showing('m2')
  session.showsScreenAt(600, 1304)
  expect(shown()).toEqual([
    { type: 'shown', device: 'm1', width: 860, height: 1868 },
    { type: 'shown', device: 'm1', width: 600, height: 1304 },
    { type: 'shown', device: 'm2', width: 600, height: 1304 },
  ])
  session.close()
})

test('the view follows whether the watched device’s screen is off, and of no other device, and Wake asks the watched one to turn on', () => {
  const streams: UserStreamHandlers[] = []
  const sent: LiveViewerMessage[] = []
  const session = viewing(sent, streams)
  session.showing('m1')
  streams[0]!.data(framed({ type: 'screen', device: 'm1', on: false }))
  expect(session.state.screenOff).toBe(true)
  streams[0]!.data(framed({ type: 'screen', device: 'm2', on: true }))
  expect(session.state.screenOff).toBe(true)
  streams[0]!.data(framed({ type: 'screen', device: 'm1', on: true }))
  expect(session.state.screenOff).toBe(false)
  session.wake()
  expect(sent.filter((message) => message.type === 'wake')).toEqual([{ type: 'wake', device: 'm1' }])
  session.close()
})

/**
 * A controller over a device m1 whose live view is open, and whose rotations
 * wait until the test answers each, in turn: `answer` gives the next one the
 * device's answer and waits for its press to end, `refuse` its refusal.
 * `reports` sends the device's notice that its pictures now come in an
 * orientation, as the module announces each generation, whoever turned it.
 */
function rotating() {
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
    { pictures: async () => false, platform: 'mac' },
  ))!
  const session = controller.session
  const presses: Array<Promise<unknown>> = []
  let generation = 0
  const open = () => {
    session.start()
    streams.at(-1)!.data(framed({
      type: 'state',
      devices: [{ id: 'm1', name: 'Pixel 10', platform: 'android', os: 'Android 16', model: 'Pixel 10', recording: false, orientation: 'portrait' as const }],
      watched: 'm1',
    }))
  }
  open()
  const reports = (orientation: Orientation) => {
    generation += 1
    const upright = orientation === 'portrait' || orientation === 'portrait-upside-down'
    streams.at(-1)!.data(framed({
      type: 'stream', device: 'm1', generation,
      width: upright ? 1080 : 2424, height: upright ? 2424 : 1080,
      pointsWidth: upright ? 412 : 924, pointsHeight: upright ? 924 : 412,
      orientation, scale: 1,
    }))
  }
  return {
    session,
    reported,
    reports,
    /** The connection drops and opens again: its generations start over. */
    reconnect: async () => {
      const before = streams.length
      streams.at(-1)!.closed('the connection dropped')
      generation = 0
      // The page's return opens it again without the rest of its wait.
      await pageReturned()
      expect(streams).toHaveLength(before + 1)
      streams.at(-1)!.data(framed({
        type: 'state',
        devices: [{ id: 'm1', name: 'Pixel 10', platform: 'android', os: 'Android 16', model: 'Pixel 10', recording: false, orientation: 'portrait' as const }],
        watched: 'm1',
      }))
    },
    rotate: (turn: 'left' | 'right') => {
      presses.push(controller.run({ method: 'rotate', params: { device: 'm1', turn } }))
    },
    /** The quarter turns the view adds to the picture it has drawn, of `drawn`. */
    quarters: (drawn: Orientation) => session.pendingQuarters('m1', drawn),
    answer: async (orientation: Orientation) => {
      waiting.shift()!.resolve({ device: 'm1', orientation, width: 1, height: 1 })
      await presses.shift()
    },
    refuse: async () => {
      waiting.shift()!.reject(new Error('the device did not turn'))
      await presses.shift()
    },
    end: () => {
      session.close()
      controller.dispose()
      scope.stop()
    },
  }
}

test('a rotation turns the drawn picture at once until a picture in the device’s new orientation is drawn, however late it answers', async () => {
  const device = rotating()
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
  const device = rotating()
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
  const device = rotating()
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
  const device = rotating()
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
  const device = rotating()
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
  const device = rotating()
  for (const orientation of ['portrait', 'landscape-right', 'portrait-upside-down', 'landscape-left', 'portrait'] as const) {
    device.reports(orientation)
  }
  device.rotate('right')
  await device.answer('landscape-right')
  await device.reconnect()
  // Nothing the old connection reported turns the picture until the new one reports.
  expect(device.quarters('portrait')).toBe(0)
  // The new connection's generations start at 1.
  device.reports('landscape-right')
  expect(device.quarters('landscape-right')).toBe(0)
  expect(device.session.state.turning).toEqual([])
  device.end()
})

test('a press before the device reported its orientation leaves later presses alone', async () => {
  const device = rotating()
  device.rotate('right')
  expect(device.quarters('portrait')).toBe(0)
  device.reports('portrait')
  device.rotate('right')
  expect(device.quarters('portrait')).toBe(1)
  // The first press's answer is no answer to the second.
  await device.answer('landscape-right')
  expect(device.quarters('portrait')).toBe(1)
  device.reports('landscape-right')
  await device.answer('landscape-right')
  expect(device.quarters('landscape-right')).toBe(0)
  expect(device.session.state.turning).toEqual([])
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
    panel: { tabs: () => [], select: () => {} },
    openSettings: unused,
    openLink: unused,
    overlays: { open: unused } as never,
    pinia: createPinia(),
  }
}

const IPHONE = { id: 'm1', name: 'iPhone 17', platform: 'ios' as const, os: 'iOS 26.5', model: 'iPhone 17', recording: false, orientation: 'portrait' as const }
const PORTRAIT: DeviceStream = { device: 'm1', generation: 1, width: 804, height: 1748, pointsWidth: 402, pointsHeight: 874, orientation: 'portrait' }
const ROUND = { kind: 'round' as const, radius: 55 }
const DRAWN = { kind: 'drawn' as const, corners: { topLeft: ROUND, topRight: ROUND, bottomRight: ROUND, bottomLeft: ROUND }, buttons: [] }

/**
 * Planted defects this catches: a conversation's simulators that end with
 * its panel, as the panel session did, which drop the picture and frame and
 * open a new connection on return, and a view that stays watched with no
 * device tab shown, which keeps the conversation awake.
 */
test('a return to a conversation shows its device’s kept picture and frame at once, on the connection it kept', async () => {
  const streams: UserStreamHandlers[] = []
  const sent: LiveViewerMessage[] = []
  const told: Array<boolean | 'closed'> = []
  const host = storeHost(plugin([], streams, sent, async (method) => ANSWERS[method], told))
  const page = pageContext(host, definePage({ plugin: 'mobile' }))
  const devices = page.store(mobileStore({ pictures: async () => true, platform: 'mac' }))

  // The panel shows the device's tab: its content watches the conversation's simulators.
  const shown = devices.watch('c1')
  const session = shown.session.devices.session
  await waitFor(() => shown.session.devices.pictures.value === 'supported')
  session.showing('m1')
  session.frame(800, 1200, 2)
  streams[0]!.data(framed({ type: 'state', devices: [IPHONE], watched: 'm1' }))
  streams[0]!.data(framed({ type: 'frame', device: 'm1', width: 800, height: 1200, scale: 2, orientation: 'portrait', pointsWidth: 402, pointsHeight: 874, frame: DRAWN }))
  const picture = { image: { width: 804, height: 1748 } as never, stream: PORTRAIT }
  // The user switches to another conversation: the content unmounts, handing on its picture.
  session.keepPicture('m1', picture)
  session.hiding('m1')
  shown.end()
  await Promise.resolve()
  expect(watches(sent).at(-1)).toBeNull()
  expect(told).toEqual([false])
  devices.watch('c2').end()

  // And back: the same simulators, the picture and frame at once, and the same connection watching again.
  const back = devices.watch('c1')
  expect(back.session.devices.session).toBe(session)
  expect(session.keptPicture('m1')).toBe(picture)
  expect(session.state.frames.m1?.frame).toEqual(DRAWN)
  session.showing('m1')
  expect(streams).toHaveLength(1)
  expect(told).toEqual([false, true])
  expect(watches(sent).at(-1)).toBe('m1')
  back.end()
  // Turned off, the plugin's store ends the connection.
  disposePluginStores(host.pinia, 'mobile')
  expect(told.at(-1)).toBe('closed')
})

/** A disk cache over a map, as the plugin's IndexedDB keeps it. */
function mapCache(kept: Map<string, Uint8Array<ArrayBuffer>>): PluginCache {
  return {
    read: async (key) => kept.get(key) ?? null,
    write: async (key, bytes) => void kept.set(key, bytes),
  }
}

/**
 * A session over the disk cache `kept` that watches m1, a `device`, in an
 * area `size` × 1200 device pixels at `scale`, its pictures portrait, or
 * none announced yet without `pictures`.
 */
function cachedSession(kept: Map<string, Uint8Array<ArrayBuffer>>, device: LiveDevice = IPHONE, size = 800, scale = 2, pictures = true) {
  const streams: UserStreamHandlers[] = []
  const session = new MobileSession({
    open: plugin([], streams).stream('mobile'),
    cache: mapCache(kept),
    platform: 'mac',
    defect: (_, error) => { throw error },
  })
  session.enable()
  session.showing('m1')
  session.frame(size, 1200, scale)
  streams[0]!.data(framed({ type: 'state', devices: [device], watched: 'm1' }))
  if (pictures) {
    streams[0]!.data(framed({ type: 'stream', device: 'm1', generation: 1, width: 804, height: 1748, pointsWidth: 402, pointsHeight: 874, orientation: 'portrait', scale: 1 }))
  }
  return { session, host: streams[0]! }
}

/** Lets the map cache's reads under way complete. */
async function cacheReads(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
}

/**
 * Planted defects this catches: a frame cached under a key without the size,
 * the model or the orientation, which would show one device's frame drawn for
 * another size or model, and none read back at all.
 */
test('a device shows the frame the disk cache keeps for its model, size, pixel ratio and orientation at once, after a reload', async () => {
  const kept = new Map<string, Uint8Array<ArrayBuffer>>()
  const open = (device = IPHONE, size = 800, scale = 2) => cachedSession(kept, device, size, scale)
  const first = open()
  expect(first.session.state.frames.m1).toBeUndefined()
  first.host.data(framed({ type: 'frame', device: 'm1', width: 800, height: 1200, scale: 2, orientation: 'portrait', pointsWidth: 402, pointsHeight: 874, frame: DRAWN }))
  await waitFor(() => kept.size === 1)
  first.session.close()

  // A reload: the same model at the same size shows the frame before the Host draws it.
  const again = open()
  await waitFor(() => again.session.state.frames.m1 !== undefined)
  expect(again.session.state.frames.m1).toEqual({ device: 'm1', orientation: 'portrait', pointsWidth: 402, pointsHeight: 874, frame: DRAWN })
  again.session.close()
  // Another size, pixel ratio or model waits for the Host's.
  for (const other of [open(IPHONE, 1000), open(IPHONE, 800, 1), open({ ...IPHONE, model: 'iPhone 17 Pro' })]) {
    await cacheReads()
    expect(other.session.state.frames.m1).toBeUndefined()
    other.session.close()
  }
})

/**
 * A reloaded page's pictures start only once it laid the screen out, for
 * which it needs a frame, so the cached one shows before any picture, in the
 * orientation the device's screen reports, and the Host's own is still under
 * way. Planted defect this catches: a cached frame that waits for the
 * pictures' orientation, which leaves the view connecting until the Host
 * draws its frame.
 */
test('a reloaded view shows the cached frame before the device sends a picture or the Host its frame', async () => {
  const kept = new Map<string, Uint8Array<ArrayBuffer>>()
  const first = cachedSession(kept)
  first.host.data(framed({ type: 'frame', device: 'm1', width: 800, height: 1200, scale: 2, orientation: 'portrait', pointsWidth: 402, pointsHeight: 874, frame: DRAWN }))
  await waitFor(() => kept.size === 1)
  first.session.close()

  const reloaded = cachedSession(kept, IPHONE, 800, 2, false)
  await waitFor(() => reloaded.session.state.frames.m1 !== undefined)
  expect(reloaded.session.state.frames.m1).toEqual({ device: 'm1', orientation: 'portrait', pointsWidth: 402, pointsHeight: 874, frame: DRAWN })
  reloaded.session.close()
  // A device whose screen reports another orientation waits for the Host's frame.
  const turned = cachedSession(kept, { ...IPHONE, orientation: 'landscape-left' as const }, 800, 2, false)
  await cacheReads()
  expect(turned.session.state.frames.m1).toBeUndefined()
  turned.session.close()
})

/**
 * Planted defects this catches: a frame kept under the size the view has when
 * it arrives rather than the size its request named, which a later view of
 * the new size draws at the wrong size, and a frame read for a size the view
 * left before the read completed, which it then shows.
 */
test('a panel resized while a frame is under way keeps and shows each frame only for the size it was drawn for', async () => {
  const kept = new Map<string, Uint8Array<ArrayBuffer>>()
  const resized = cachedSession(kept)
  resized.session.frame(1000, 1200, 2)
  resized.host.data(framed({ type: 'frame', device: 'm1', width: 800, height: 1200, scale: 2, orientation: 'portrait', pointsWidth: 402, pointsHeight: 874, frame: DRAWN }))
  await waitFor(() => kept.size === 1)
  resized.session.close()

  const wider = cachedSession(kept, IPHONE, 1000)
  await cacheReads()
  expect(wider.session.state.frames.m1).toBeUndefined()
  wider.session.close()

  // The read of the 800 pixels' frame is under way as the panel widens.
  const widening = cachedSession(kept)
  widening.session.frame(1000, 1200, 2)
  await cacheReads()
  expect(widening.session.state.frames.m1).toBeUndefined()
  widening.session.close()

  const same = cachedSession(kept)
  await waitFor(() => same.session.state.frames.m1 !== undefined)
  expect(same.session.state.frames.m1?.frame).toEqual(DRAWN)
  same.session.close()
})

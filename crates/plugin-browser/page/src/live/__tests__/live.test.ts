import { expect, jest, spyOn, test } from 'bun:test'
import type { LiveControl, LiveModuleMessage, LiveTab, LiveViewerMessage } from '../../generated/plugin'
import { LiveFrameReader, encodeFile, encodeMessage } from '../frames'
import { browserShortcut, inputMessage, localKey, shortcutKeys } from '../input'
import { LiveFrameSplitter, decodeAcks, encodeVideo, pageReturned, pointerInput, type LiveChunkAck } from '@demicodes/plugin-sdk'
import { LIVE_CHUNK_DATA_BYTES } from '@demicodes/protocol'
import type { UserStreamHandlers } from '@demicodes/plugin-sdk'
import { LiveSession, REFUSED_FRAME, SILENT_STREAM, type LiveSessionOptions, type LiveStream, type PanelReport, type PictureSink } from '../session'
import { cursorAt, deviceSnap, panelSize, placePicture, panelRect, tabPoint, viewportChoices } from '../view'

const WEB = { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' } as const
const PHONE = { width: 390, height: 844, devicePixelRatio: 2, mode: 'mobile' } as const

/** A module frame as the Host writes it. */
function video(tab: string, generation: number, sequence: number, key: boolean, data: number[]): Uint8Array {
  return encodeVideo({
    source: tab,
    generation,
    sequence,
    key,
    timestamp: 1234,
    width: 1600,
    height: 1200,
    data: new Uint8Array(data),
  })
}

/** A frame of any kind and payload, as a module that breaks the protocol might write it. */
function rawFrame(kind: number, payload: Uint8Array): Uint8Array {
  const frame = new Uint8Array(5 + payload.length)
  new DataView(frame.buffer).setUint32(0, 1 + payload.length)
  frame[4] = kind
  frame.set(payload, 5)
  return frame
}

function moduleFrame(message: LiveModuleMessage): Uint8Array {
  return rawFrame(1, new TextEncoder().encode(JSON.stringify(message)))
}

test('a control message the browser\'s protocol refuses is an error, never a message', () => {
  const text = (value: string) => new TextEncoder().encode(value)
  const refused: Array<[Uint8Array, string]> = [
    [rawFrame(1, text('{"type":')), 'not JSON'],
    [rawFrame(1, new Uint8Array([0x7b, 0xff, 0x7d])), 'not JSON'],
    [rawFrame(1, text('{"type":"state","running":"yes","tabs":[],"watched":null}')), 'the protocol refuses'],
  ]
  for (const [bytes, reason] of refused) {
    expect(() => new LiveFrameReader().read(bytes)).toThrow(reason)
  }
})

test('the viewer\'s own paste keys stay with its browser, and reach the page as a paste', () => {
  const key = (letter: string, altGraph = false) => ({
    altKey: altGraph, ctrlKey: altGraph, metaKey: !altGraph, shiftKey: false,
    key: letter, code: `Key${letter.toUpperCase()}`, keyCode: 0, repeat: false, location: 0,
    getModifierState: (name: string) => name === 'AltGraph' && altGraph,
  })
  expect(localKey(key('v'))).toBe(true)
  expect(localKey(key('c'))).toBe(false)
  expect(localKey(key('v', true))).toBe(false)
})

test('a viewer message and a file frame carry their kind and header', () => {
  const message = encodeMessage({ type: 'watch', tab: null })
  expect(new DataView(message.buffer).getUint32(0)).toBe(message.length - 4)
  expect(message[4]).toBe(1)
  const file = encodeFile(7, 1, new Uint8Array([1, 2, 3]))
  expect(file[4]).toBe(4)
  const header = new DataView(file.buffer, 5)
  expect([header.getUint32(0), header.getUint32(4)]).toEqual([7, 1])
  expect([...file.subarray(13)]).toEqual([1, 2, 3])
})

/** A generation of `viewport`'s pictures, at the size the module encodes them. */
function generation(viewport: LiveStream['viewport'], width = viewport.width * viewport.devicePixelRatio, height = viewport.height * viewport.devicePixelRatio): LiveStream {
  return { tab: 't1', generation: 1, width, height, viewport, scale: 1 }
}

test('a web picture stands unscaled at the top-left corner, and a phone keeps its size in the middle', () => {
  const panel = panelSize(800.4, 600.6)
  expect(panel).toEqual({ width: 800, height: 601 })
  // The panel grew: the picture of the old size stands as it is, the rest is the frame's white.
  expect(placePicture(generation(WEB), { width: 1000, height: 700 })).toEqual({ scale: 1, left: 0, top: 0, width: 800, height: 600 })
  // The panel shrank: the panel cuts the picture.
  expect(placePicture(generation(WEB), { width: 500, height: 400 })).toEqual({ scale: 1, left: 0, top: 0, width: 800, height: 600 })
  // An odd width at ratio 1 is encoded a pixel narrower, and shown a pixel narrower, never stretched.
  const odd = generation({ width: 701, height: 401, devicePixelRatio: 1, mode: 'web' }, 700, 400)
  expect(placePicture(odd, { width: 701, height: 401 })).toMatchObject({ scale: 1, width: 700, height: 400 })
  const phone = placePicture(generation(PHONE), { width: 800, height: 600 })
  expect(phone.scale).toBeCloseTo(600 / 844)
  expect(phone.left).toBeCloseTo((800 - 390 * phone.scale) / 2)
  expect(phone.top).toBe(0)
})

test('input maps by the picture shown, never by a viewport the tab list reports before its picture', () => {
  // The tab list already says Mobile; the picture shown is still the Web one.
  const shown = placePicture(generation(WEB), { width: 800, height: 600 })
  expect(tabPoint({ x: 400, y: 300 }, WEB, shown)).toEqual({ x: 400, y: 300 })
  const next = placePicture(generation(PHONE), { width: 800, height: 600 })
  expect(tabPoint({ x: 400, y: 300 }, PHONE, next).x).toBeCloseTo(195, 0)
})

test('the cursor under the pointer comes from the page\'s regions, the last one over the point first', () => {
  const regions = [
    { x: 0, y: 0, width: 400, height: 300, cursor: 'pointer' },
    { x: 100, y: 100, width: 50, height: 50, cursor: 'text' },
    { x: 200, y: 100, width: 50, height: 50, cursor: 'auto' },
  ]
  const cases: Array<[string, { x: number; y: number }, string, string]> = [
    ['over a button', { x: 10, y: 10 }, 'default', 'pointer'],
    ['over a field in it', { x: 120, y: 120 }, 'default', 'text'],
    ['where the page leaves it to the browser', { x: 220, y: 120 }, 'text', 'text'],
    ['outside every region', { x: 500, y: 500 }, 'default', 'default'],
    ['where the observer resolved a name this browser has no rule for', { x: 500, y: 500 }, 'url(x.png)', 'default'],
  ]
  for (const [where, point, resolved, cursor] of cases) {
    expect({ where, cursor: cursorAt(point, regions, resolved) }).toEqual({ where, cursor })
  }
})

test('a picture starts on a whole device pixel wherever its panel stands', () => {
  expect(deviceSnap(600, 2)).toBe(0)
  expect(deviceSnap(600.5, 2)).toBe(0)
  // A percentage split leaves the panel between two device pixels.
  expect(deviceSnap(594.67, 2)).toBeCloseTo(-0.17, 2)
  expect(deviceSnap(594.3, 1)).toBeCloseTo(-0.3, 5)
  expect((594.67 + deviceSnap(594.67, 2)) * 2).toBeCloseTo(1189, 5)
})

test('panel points map to the tab and back, within the picture', () => {
  const placement = placePicture(generation(PHONE), { width: 800, height: 600 })
  const middle = tabPoint({ x: placement.left + placement.width / 2, y: placement.height / 2 }, PHONE, placement)
  expect(middle.x).toBeCloseTo(195)
  expect(middle.y).toBeCloseTo(422)
  // Outside the picture the pointer stays on its edge.
  expect(tabPoint({ x: 0, y: 0 }, PHONE, placement)).toEqual({ x: 0, y: 0 })
  const control = panelRect({ x: 10, y: 20, width: 100, height: 30 }, placement)
  expect(control.left).toBeCloseTo(placement.left + 10 * placement.scale)
  expect(control.width).toBeCloseTo(100 * placement.scale)
})

test('the viewport menu offers Web and Mobile, and shows what the agent set', () => {
  expect(viewportChoices(WEB).map((choice) => choice.label)).toEqual(['Web', 'Mobile'])
  const custom = viewportChoices({ width: 1440, height: 900, devicePixelRatio: 2, mode: 'custom' })
  expect(custom.map((choice) => choice.label)).toEqual(['Web', 'Mobile', 'Custom 1440 × 900 @2'])
  expect(custom.at(-1)!.selectable).toBe(false)
})

/** A session over a stream the test drives, with a clock it controls. */
function session(options: Partial<LiveSessionOptions> = {}) {
  const sent: LiveViewerMessage[] = []
  const acks: LiveChunkAck[] = []
  const pictures: Array<[string, number, number]> = []
  const decoder = new TextDecoder()
  let handlers: UserStreamHandlers | null = null
  /** What each stream, by its number from 1, was told of who watches it. */
  const told: Array<[number, boolean]> = []
  let streams = 0
  let now = 0
  const sink: PictureSink = {
    start: (stream) => pictures.push(['start', stream.generation, stream.width] as never) as never,
    show: (frame) => pictures.push(['show', frame.generation, frame.sequence] as never) as never,
    stop: () => pictures.push(['stop', 0, 0] as never) as never,
  }
  const live = new LiveSession({
    open: (given: UserStreamHandlers) => {
      handlers = given
      const stream = ++streams
      return {
        send: (bytes) => {
          if (bytes[4] === 1) {
            sent.push(JSON.parse(decoder.decode(bytes.subarray(5))) as LiveViewerMessage)
          } else if (bytes[4] === 3) {
            acks.push(...decodeAcks(bytes.subarray(5)))
          }
        },
        watched: (watched) => void told.push([stream, watched]),
        close: () => handlers?.closed('closed'),
      }
    },
    platform: 'mac',
    panel: () => null,
    now: () => now,
    // The defects these tests plant are the frames and messages they check the view's answer to.
    defect: () => {},
    ...options,
  })
  live.attach(sink)
  live.start()
  return {
    live,
    sent,
    acks,
    pictures,
    told,
    receive: (bytes: Uint8Array) => handlers!.data(bytes),
    close: (reason: string) => handlers!.closed(reason),
    advance: (ms: number) => {
      now += ms
    },
  }
}

const TAB: LiveTab = {
  id: 't1',
  title: 'Example',
  url: 'https://example.test/',
  createdBy: { kind: 'agent', number: 0 },
  viewport: { ...WEB },
  loading: false,
  canGoBack: false,
  canGoForward: false,
}

test('a view says hello, learns the tabs and acknowledges each chunk with when it came', async () => {
  const view = session()
  expect(view.sent[0]).toEqual({ type: 'hello', platform: 'mac' })
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  expect(view.live.state).toMatchObject({ connection: 'live', running: true, watched: TAB.id })
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
  view.advance(5)
  view.receive(video(TAB.id, 1, 4, true, [0, 0, 0, 1]))
  // Frames of an older generation are not shown, but acknowledged, which frees the Host's window.
  view.receive(video(TAB.id, 0, 5, true, [0, 0, 0, 1]))
  expect(view.pictures).toEqual([['start', 1, 1600], ['show', 1, 4]] as never)
  await Promise.resolve()
  expect(view.acks).toEqual([
    { generation: 1, sequence: 4, chunk: 0, received: 5 },
    { generation: 0, sequence: 5, chunk: 0, received: 5 },
  ])
})

test('a canvas that comes after its stream started shows it from a key frame it asks for', () => {
  // After a reload the stream starts, and a still page sends its one key frame, before the view has a canvas.
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 2, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
  view.receive(video(TAB.id, 2, 1, true, [0, 0, 0, 1]))
  const started: number[] = []
  view.live.attach({ start: (stream) => started.push(stream.generation), show: () => {}, stop: () => {} })
  expect(started).toEqual([2])
  expect(view.sent.at(-1)).toEqual({ type: 'keyframe', generation: 2 })
  // The canvas of a tab the view moves to does not start on the pictures of the one it left.
  const other = { ...TAB, id: 't2' }
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB, other], watched: TAB.id }))
  view.live.watch(other.id)
  view.live.attach({ start: (stream) => started.push(stream.generation), show: () => {}, stop: () => {} })
  expect(started).toEqual([2])
})

test('a stalled stream discards input and resumes where it was', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
  view.advance(300)
  view.live.tick()
  expect(view.live.state.connection).toBe('live')
  view.advance(1200)
  view.live.tick()
  expect(view.live.state.connection).toBe('stalled')
  expect(view.sent.at(-1)).toEqual({ type: 'release' })
  const before = view.sent.length
  view.live.input(inputMessage(TAB.id, pointerInput('down', { x: 1, y: 2 }, { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, button: 0, buttons: 1 })))
  expect(view.sent).toHaveLength(before)
  view.receive(moduleFrame({ type: 'heartbeat' }))
  expect(view.live.state.connection).toBe('live')
  // Every chunk still arrives, so the decoder needs no key frame.
  expect(view.sent).toHaveLength(before)
})

test('a choice names the revision the viewer saw, and a dialog is answered once', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  const control: LiveControl = {
    token: '00000000-0000-4000-8000-000000000000',
    revision: 3,
    kind: 'select',
    label: 'choice',
    value: 'a',
    min: '', max: '', step: '', accept: '',
    multiple: false, disabled: false, required: false, size: 0,
    options: [],
    rect: { x: 0, y: 0, width: 10, height: 10 },
  }
  // Rejoining a live tab sends its unchanged controls before the first stream.
  view.receive(moduleFrame({ type: 'controls', tab: TAB.id, controls: [control] }))
  for (const generation of [1, 2]) {
    view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
    expect(view.live.state.controls).toEqual([control])
  }
  view.live.choose(control, 'b', [1])
  expect(view.sent.at(-1)).toMatchObject({ type: 'choice', token: control.token, revision: 3, value: 'b', indices: [1] })
  view.receive(moduleFrame({ type: 'dialog', tab: TAB.id, dialog: { type: 'prompt', message: 'name?', defaultText: '' } }))
  expect(view.live.state.dialog?.dialog.type).toBe('prompt')
  view.live.answerDialog(true, 'demi')
  expect(view.sent.at(-1)).toEqual({ type: 'dialog', tab: TAB.id, accept: true, text: 'demi' })
  // The first answer wins: a second does not reach the module.
  const after = view.sent.length
  view.live.answerDialog(false)
  expect(view.sent).toHaveLength(after)
  view.live.watch(null)
  expect(view.live.state.controls).toEqual([])
})

test('the view ends with the reason the module or the backend gave', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'ended', reason: 'browser_ended' }))
  expect(view.live.state).toMatchObject({ connection: 'opening', ended: 'browser_ended' })
  expect(view.pictures.at(-1)).toEqual(['stop', 0, 0] as never)
  view.live.close()
})

test('a view that ends opens again after the page\'s waits, which start over once a view works', () => {
  jest.useFakeTimers()
  // Without the random part: a second, then twice as long each time, up to 30 seconds.
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    const view = session()
    const hellos = () => view.sent.filter((message) => message.type === 'hello').length
    const reopenWait = () => {
      const count = hellos()
      view.close('host_unreachable')
      let waited = 0
      while (hellos() === count) {
        jest.advanceTimersByTime(100)
        waited += 100
      }
      return waited
    }
    const waits: number[] = []
    for (let ends = 0; ends < 6; ends += 1) {
      waits.push(reopenWait())
    }
    expect(waits).toEqual([1_000, 2_000, 4_000, 8_000, 16_000, 30_000])
    view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
    expect(reopenWait()).toBe(1_000)
    view.live.close()
  } finally {
    random.mockRestore()
    jest.useRealTimers()
  }
})

/**
 * Planted defects this catches: an unwatched view that opens again by itself,
 * which would wake the Cloud the backend let stop, and one that stays ended
 * once watched.
 */
test('a view nobody watches that the backend ends opens again only once watched, saying who watches each stream', () => {
  jest.useFakeTimers()
  try {
    const view = session()
    const hellos = () => view.sent.filter((message) => message.type === 'hello').length
    view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
    view.live.watched(false)
    expect(view.told).toEqual([[1, false]])
    view.close('host_unreachable')
    jest.advanceTimersByTime(60_000)
    expect(hellos()).toBe(1)
    view.live.watched(true)
    expect(hellos()).toBe(2)
    // The new stream starts watched, as every stream does, and is told nothing more.
    expect(view.told).toEqual([[1, false]])
    view.live.close()
  } finally {
    jest.useRealTimers()
  }
})

/**
 * Planted defects this catches: hiding a view that waits to open again
 * leaving its wait, or a reconnect, to open it in the background.
 */
test('a view that ended while watched and is then hidden opens again only once watched', () => {
  jest.useFakeTimers()
  try {
    const view = session()
    const hellos = () => view.sent.filter((message) => message.type === 'hello').length
    view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
    view.close('host_unreachable')
    view.live.watched(false)
    jest.advanceTimersByTime(60_000)
    view.live.reconnect()
    expect(hellos()).toBe(1)
    view.live.watched(true)
    expect(hellos()).toBe(2)
    view.live.close()
  } finally {
    jest.useRealTimers()
  }
})

test('a view whose stream brings nothing for 75 seconds is broken and opens again, at once when the page comes back', () => {
  jest.useFakeTimers()
  // Without the random part: the first wait is a second.
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    const ended: string[] = []
    const view = session({ onEnded: (reason) => ended.push(reason) })
    const hellos = () => view.sent.filter((message) => message.type === 'hello').length
    view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
    // A still page: the module's heartbeats are all the view hears, and they keep it.
    for (let beats = 0; beats < 3; beats += 1) {
      jest.advanceTimersByTime(74_000)
      view.receive(moduleFrame({ type: 'heartbeat' }))
    }
    // The network drops without a close, and nothing more arrives.
    jest.advanceTimersByTime(74_999)
    expect(ended).toEqual([])
    jest.advanceTimersByTime(1)
    // The judgement waits for the messages already queued.
    jest.advanceTimersByTime(0)
    expect(ended).toEqual([SILENT_STREAM])
    expect(view.live.state).toMatchObject({ connection: 'opening', ended: SILENT_STREAM })
    jest.advanceTimersByTime(1_000)
    expect(hellos()).toBe(2)
    view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
    // The laptop sleeps for 80 seconds with the page shown: the clock goes
    // on, the watch's timer does not, and the page comes back online.
    jest.setSystemTime(Date.now() + 80_000)
    pageReturned()
    jest.advanceTimersByTime(0)
    expect(ended).toEqual([SILENT_STREAM, SILENT_STREAM])
    expect(hellos()).toBe(3)
    view.live.close()
    // A closed view's watch is gone with its stream.
    jest.advanceTimersByTime(75_000)
    expect(ended).toHaveLength(2)
  } finally {
    random.mockRestore()
    jest.useRealTimers()
  }
})

test('a view that ends opens again, watching what the viewer watched', () => {
  jest.useFakeTimers()
  try {
    const report: PanelReport = { panel: { width: 800, height: 600 }, devicePixelRatio: 2, screen: { width: 1440, height: 900 } }
    const view = session({ panel: () => report })
    view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
    view.close('host_unreachable')
    expect(view.live.state).toMatchObject({ connection: 'opening', running: false })
    expect(view.live.state.tabs).toHaveLength(0)
    jest.runOnlyPendingTimers()
    const opened = view.sent.slice(-3)
    expect(opened).toEqual([
      { type: 'hello', platform: 'mac' },
      { type: 'panel', width: 800, height: 600, devicePixelRatio: 2, screenWidth: 1440, screenHeight: 900 },
      { type: 'watch', tab: TAB.id },
    ])
    // The page stops asking once the view itself is closed.
    view.live.close()
    view.close('closed')
    jest.runOnlyPendingTimers()
    expect(view.sent.at(-1)).toEqual({ type: 'watch', tab: TAB.id })
    expect(view.live.state.connection).toBe('ended')
  } finally {
    jest.useRealTimers()
  }
})

test('a frame the protocol refuses ends the view, and the next view opens', () => {
  jest.useFakeTimers()
  try {
    const ended: string[] = []
    const view = session({ onEnded: (reason) => ended.push(reason) })
    view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
    view.receive(rawFrame(1, new TextEncoder().encode('{"type":"state","running":"yes"}')))
    expect(ended).toEqual([REFUSED_FRAME])
    expect(view.live.state).toMatchObject({ connection: 'opening', running: false, ended: REFUSED_FRAME })
    jest.runOnlyPendingTimers()
    expect(view.sent.filter((message) => message.type === 'hello')).toHaveLength(2)
    view.live.close()
  } finally {
    jest.useRealTimers()
  }
})

test('a view that opens again reads its stream from the first byte', () => {
  jest.useFakeTimers()
  try {
    const view = session()
    const state = moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id })
    // The stream ends inside a frame; the next stream's first frame is whole.
    view.receive(state.subarray(0, 7))
    view.close('host_unreachable')
    jest.runOnlyPendingTimers()
    view.receive(state)
    expect(view.live.state).toMatchObject({ connection: 'live', running: true, watched: TAB.id })
    view.live.close()
  } finally {
    jest.useRealTimers()
  }
})

test('a view the module ended and the socket then closed opens again once', () => {
  jest.useFakeTimers()
  try {
    const view = session()
    view.receive(moduleFrame({ type: 'ended', reason: 'browser_ended' }))
    view.close('closed')
    jest.runOnlyPendingTimers()
    expect(view.sent.filter((message) => message.type === 'hello')).toHaveLength(2)
    view.live.close()
  } finally {
    jest.useRealTimers()
  }
})

test('a notice that leaves no picture stands in its place until a picture; any other is told once', () => {
  const told: string[] = []
  const view = session({ onNotice: (code) => told.push(code) })
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
  view.receive(moduleFrame({ type: 'notice', code: 'capture_failed', message: 'the capture extension did not connect' }))
  expect(view.live.state.pictureless).toBe('capture_failed')
  view.receive(video(TAB.id, 1, 1, true, [0, 0, 0, 1]))
  expect(view.live.state.pictureless).toBeNull()
  view.receive(moduleFrame({ type: 'notice', code: 'driver_error', message: 'the page went away' }))
  view.receive(moduleFrame({ type: 'notice', code: 'timeout', message: 'screen: timed out' }))
  view.receive(video(TAB.id, 1, 2, false, [0, 0, 0, 1]))
  expect(view.live.state.pictureless).toBeNull()
  expect(told).toEqual(['driver_error', 'timeout'])
  // A Host that cannot capture says so for each tab the page watches.
  view.receive(moduleFrame({ type: 'notice', code: 'capture_unavailable', message: 'this CPU reports SME without SVE' }))
  expect(view.live.state.pictureless).toBe('capture_unavailable')
  view.live.watch(null)
  expect(view.live.state.pictureless).toBeNull()
  expect(told).toEqual(['driver_error', 'timeout'])
})

test('a Host that stopped trying to capture is asked again by Retry, and says it tries until a picture', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
  // Retry is only for a Host that stopped: one still trying is not asked.
  view.receive(moduleFrame({ type: 'notice', code: 'capture_failed', message: 'capture extension disconnected' }))
  view.live.recapture()
  expect(view.sent.filter((message) => message.type === 'recapture')).toHaveLength(0)
  view.receive(moduleFrame({ type: 'notice', code: 'capture_stopped', message: 'the capture extension could not be recreated' }))
  expect(view.live.state.pictureless).toBe('capture_stopped')
  view.live.recapture()
  expect(view.sent.filter((message) => message.type === 'recapture')).toHaveLength(1)
  expect(view.live.state.pictureless).toBe('capture_failed')
  view.receive(video(TAB.id, 1, 1, true, [0, 0, 0, 1]))
  expect(view.live.state.pictureless).toBeNull()
})

test('a message the protocol refuses is never sent, so the module does not end the view', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  // A DOM event's fields are getters on its prototype: a spread copy of one has none of them.
  class Hover {
    get button() { return 0 }
    get buttons() { return 0 }
    get altKey() { return false }
    get ctrlKey() { return false }
    get metaKey() { return false }
    get shiftKey() { return false }
  }
  const event = new Hover()
  const before = view.sent.length
  // Typed as the event, as the DOM's own types are; at run time the copy is empty.
  view.live.input(inputMessage(TAB.id, pointerInput('move', { x: 10, y: 20 }, Object.assign({}, event))))
  expect(view.sent).toHaveLength(before)
  view.live.input(inputMessage(TAB.id, pointerInput('move', { x: 10, y: 20 }, event)))
  expect(view.sent.at(-1)).toMatchObject({ type: 'input', tab: TAB.id, input: { type: 'pointer', action: 'move', buttons: 0, clickCount: 0 } })
  expect(view.live.state.connection).toBe('live')
})

test('the page decides what it watches: a tab that is still there is asked for again, a tab that went is let go', () => {
  const OTHER: LiveTab = { ...TAB, id: 't4' }
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB, OTHER], watched: null }))
  view.live.watch(OTHER.id)
  const asked = view.sent.length
  // The module's answer to an older state of things crosses the page's wish.
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB, OTHER], watched: null }))
  expect(view.live.state.watched).toBe(OTHER.id)
  expect(view.sent.slice(asked)).toEqual([{ type: 'watch', tab: OTHER.id }])
  // The tab itself went away.
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: null }))
  expect(view.live.state.watched).toBeNull()
})

test('text the watched tab copies reaches the viewer', () => {
  const copied: string[] = []
  const view = session({ onClipboard: (text) => copied.push(text) })
  view.receive(moduleFrame({ type: 'clipboard', text: 'copy me' }))
  expect(copied).toEqual(['copy me'])
})

/** A key as the viewer's browser reports it, with only the modifiers named. */
function pressed(code: string, held: Partial<Record<'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey', boolean>> = {}) {
  return {
    key: '', code, keyCode: 0, repeat: false, location: 0,
    metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...held,
    getModifierState: () => false,
  }
}

test('the browser’s own shortcuts act in the panel, as Chrome has them on each platform', () => {
  expect(browserShortcut(pressed('KeyL', { metaKey: true }), 'mac')).toBe('address')
  expect(browserShortcut(pressed('BracketLeft', { metaKey: true }), 'mac')).toBe('back')
  expect(browserShortcut(pressed('BracketRight', { metaKey: true }), 'mac')).toBe('forward')
  expect(browserShortcut(pressed('KeyR', { metaKey: true }), 'mac')).toBe('reload')
  expect(browserShortcut(pressed('KeyL', { ctrlKey: true }), 'linux')).toBe('address')
  expect(browserShortcut(pressed('ArrowLeft', { altKey: true }), 'windows')).toBe('back')
  expect(browserShortcut(pressed('ArrowRight', { altKey: true }), 'linux')).toBe('forward')
  expect(browserShortcut(pressed('F5'), 'windows')).toBe('reload')
  // The page's own keys: Control+L on a Mac, ⌘⇧R, a plain letter, ⌥← moving by words.
  expect(browserShortcut(pressed('KeyL', { ctrlKey: true }), 'mac')).toBeNull()
  expect(browserShortcut(pressed('KeyR', { metaKey: true, shiftKey: true }), 'mac')).toBeNull()
  expect(browserShortcut(pressed('KeyL'), 'mac')).toBeNull()
  expect(browserShortcut(pressed('ArrowLeft', { altKey: true }), 'mac')).toBeNull()
})

test('the browser’s menu copies in the page with the platform’s own copy keys', () => {
  expect(shortcutKeys('t1', 'c', 'mac').map((message) => message.type === 'input' && message.input.type === 'key' && [message.input.action, message.input.key, message.input.modifiers])).toEqual([
    ['down', 'c', 4],
    ['up', 'c', 4],
  ])
  const [cut] = shortcutKeys('t1', 'x', 'linux')
  expect(cut).toMatchObject({ type: 'input', tab: 't1', input: { type: 'key', key: 'x', code: 'KeyX', modifiers: 2 } })
  expect(cut?.type === 'input' && cut.input).not.toHaveProperty('text')
})

/**
 * On the direct path a generation's first chunks may come on `video` before
 * its announcement on `stream`:
 * the page keeps them and shows them once it is announced. Planted defect
 * this catches: dropping them, after which the generation's deltas wait for
 * a key frame that never comes.
 */
test('chunks that come before their generation is announced are shown once it is', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
  view.receive(video(TAB.id, 2, 1, true, [0, 0, 0, 1]))
  view.receive(video(TAB.id, 2, 2, false, [0, 0, 0, 2]))
  expect(view.pictures).toEqual([['start', 1, 1600]] as never)
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 2, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
  view.receive(video(TAB.id, 2, 3, false, [0, 0, 0, 3]))
  expect(view.pictures).toEqual([['start', 1, 1600], ['start', 2, 1600], ['show', 2, 1], ['show', 2, 2], ['show', 2, 3]] as never)
  expect(view.sent.filter((message) => message.type === 'keyframe')).toEqual([])
})

/**
 * A frame waits for an earlier one at most two seconds, and the page holds
 * at most 8 MiB of frames not yet whole; past either it drops what it holds
 * and asks for a key frame. Planted defects this catches: a wait without
 * end, a hold without bound.
 */
test('a frame that waits too long, or too much held, starts the pictures again from a key frame', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
  view.receive(video(TAB.id, 1, 1, true, [0, 0, 0, 1]))
  // Frame 2 never comes; frame 3 waits for it.
  view.receive(video(TAB.id, 1, 3, false, [0, 0, 0, 3]))
  view.advance(1900)
  view.live.tick()
  expect(view.sent.filter((message) => message.type === 'keyframe')).toEqual([])
  view.advance(200)
  view.live.tick()
  expect(view.sent.filter((message) => message.type === 'keyframe')).toEqual([{ type: 'keyframe', generation: 1 }])

  // Frames of two chunks of which only the first came, past 8 MiB.
  view.advance(1000)
  const firstChunk = (sequence: number) => {
    const bytes = video(TAB.id, 1, sequence, false, new Array(LIVE_CHUNK_DATA_BYTES + 1).fill(7))
    return new LiveFrameSplitter().split(bytes)[0]!
  }
  for (let sequence = 10; sequence < 10 + Math.floor(8 * 1024 * 1024 / (LIVE_CHUNK_DATA_BYTES + 1)); sequence++)
    view.receive(firstChunk(sequence))
  expect(view.sent.filter((message) => message.type === 'keyframe')).toHaveLength(1)
  view.receive(firstChunk(1000))
  expect(view.sent.filter((message) => message.type === 'keyframe')).toHaveLength(2)
})

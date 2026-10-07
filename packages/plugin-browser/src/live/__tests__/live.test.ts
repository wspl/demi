import { expect, jest, spyOn, test } from 'bun:test'
import type { LiveControl, LiveModuleMessage, LiveTab, LiveViewerMessage } from '../../generated/plugin'
import { LiveFrameReader, encodeFile, encodeMessage, encodeVideo, type LiveFrame } from '../frames'
import { ClickCount, keyMessage, localKey, modifiers, pointerMessage, wheelMessage } from '../input'
import { pageReturned } from '@demicodes/plugin-sdk'
import type { UserStreamHandlers } from '@demicodes/plugin-sdk'
import { LiveSession, REFUSED_FRAME, SILENT_STREAM, type LiveSessionOptions, type LiveStream, type PanelReport, type PictureSink } from '../session'
import { cursorAt, deviceSnap, panelSize, placePicture, panelRect, tabPoint, viewportChoices } from '../view'

const WEB = { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' } as const
const PHONE = { width: 390, height: 844, devicePixelRatio: 2, mode: 'mobile' } as const

/** A module frame as the Host writes it. */
function video(tab: string, generation: number, sequence: number, key: boolean, data: number[]): Uint8Array {
  return encodeVideo({
    tab,
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

test('frames split across chunks and join within one', () => {
  const reader = new LiveFrameReader()
  const bytes = new Uint8Array([
    ...moduleFrame({ type: 'heartbeat' }),
    ...video('t1', 3, 7, true, [0, 0, 0, 1, 9]),
  ])
  const oneByOne: LiveFrame[] = []
  for (const byte of bytes) {
    oneByOne.push(...reader.read(new Uint8Array([byte])))
  }
  const together = new LiveFrameReader().read(bytes)
  for (const frames of [oneByOne, together]) {
    expect(frames).toHaveLength(2)
    expect(frames[0]).toEqual({ kind: 'message', message: { type: 'heartbeat' } })
    expect(frames[1]!.kind).toBe('video')
    const frame = frames[1]!.kind === 'video' ? frames[1]!.frame : null
    expect(frame).toMatchObject({ tab: 't1', generation: 3, sequence: 7, key: true, width: 1600, height: 1200 })
    expect([...frame!.data]).toEqual([0, 0, 0, 1, 9])
  }
})

test('a frame the protocol refuses is an error, never a frame', () => {
  const text = (value: string) => new TextEncoder().encode(value)
  const refused: Array<[Uint8Array, string]> = [
    [rawFrame(1, text('{"type":')), 'not JSON'],
    [rawFrame(1, new Uint8Array([0x7b, 0xff, 0x7d])), 'not JSON'],
    [rawFrame(1, text('{"type":"state","running":"yes","tabs":[],"watched":null}')), 'the protocol refuses'],
    [rawFrame(3, new Uint8Array(8)), 'kind 3'],
    [rawFrame(2, new Uint8Array([0, 0, 0, 1])), 'shorter than its header'],
    [new Uint8Array([0, 0, 0, 0]), 'impossible size'],
  ]
  for (const [bytes, reason] of refused) {
    expect(() => new LiveFrameReader().read(bytes)).toThrow(reason)
  }
})

test('a viewer message and a file frame carry their kind and header', () => {
  const message = encodeMessage({ type: 'watch', tab: null })
  expect(new DataView(message.buffer).getUint32(0)).toBe(message.length - 4)
  expect(message[4]).toBe(1)
  const file = encodeFile(7, 1, new Uint8Array([1, 2, 3]))
  expect(file[4]).toBe(3)
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

test('pointer, wheel and key events carry what the page needs', () => {
  const none = { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false }
  const down = pointerMessage('t_a', 'down', { x: 5, y: 6 }, { ...none, button: 2, buttons: 2 })
  expect(down).toMatchObject({ type: 'pointer', action: 'down', button: 'right', buttons: 2, clickCount: 1, modifiers: 0 })
  const move = pointerMessage('t_a', 'move', { x: 1, y: 2 }, { ...none, button: 0, buttons: 1 })
  expect(move).toMatchObject({ button: 'left', clickCount: 0 })
  const lines = wheelMessage('t_a', { x: 0, y: 0 }, { ...none, deltaX: 0, deltaY: 3, deltaMode: 1 }, 600)
  expect(lines).toMatchObject({ deltaY: 48 })
  const pages = wheelMessage('t_a', { x: 0, y: 0 }, { ...none, deltaX: 0, deltaY: -1, deltaMode: 2 }, 600)
  expect(pages).toMatchObject({ deltaY: -600 })
  const typed = keyMessage('t_a', 'down', { ...none, key: 'a', code: 'KeyA', keyCode: 65, repeat: false, location: 0, getModifierState: () => false })
  expect(typed).toMatchObject({ type: 'key', action: 'down', text: 'a', modifiers: 0 })
  const shortcut = keyMessage('t_a', 'down', { ...none, ctrlKey: true, key: 'c', code: 'KeyC', keyCode: 67, repeat: false, location: 0, getModifierState: () => false })
  expect(shortcut).not.toHaveProperty('text')
  expect(shortcut.type === 'key' && shortcut.modifiers).toBe(2)
  const released = keyMessage('t_a', 'up', { ...none, key: 'a', code: 'KeyA', keyCode: 65, repeat: true, location: 0, getModifierState: () => false })
  expect(released).toMatchObject({ action: 'up', repeat: false })
  expect(modifiers({ altKey: true, ctrlKey: false, metaKey: true, shiftKey: true })).toBe(1 | 4 | 8)
  // The viewer's own paste reaches the page as a paste event.
  expect(localKey({ ...none, metaKey: true, key: 'v', code: 'KeyV', keyCode: 86, repeat: false, location: 0, getModifierState: () => false })).toBe(true)
  expect(localKey({ ...none, metaKey: true, key: 'c', code: 'KeyC', keyCode: 67, repeat: false, location: 0, getModifierState: () => false })).toBe(false)
})

test('presses close in time and place count as one double or triple click, as the page needs them', () => {
  // A pointer event's own detail is 0, so the view counts.
  const clicks = new ClickCount()
  const press = (timeStamp: number, clientX = 10, button = 0) => clicks.press({ timeStamp, clientX, clientY: 10, button })
  expect([press(0), press(200), press(400), press(600)]).toEqual([1, 2, 3, 3])
  expect(press(2000)).toBe(1)
  expect(press(2100, 30)).toBe(1)
  expect(press(2200, 30, 2)).toBe(1)
  expect(clicks.current).toBe(1)
  expect(pointerMessage('t_a', 'down', { x: 1, y: 1 }, { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, button: 0, buttons: 1 }, 2))
    .toMatchObject({ clickCount: 2 })
})

test('keyboard prototype getters preserve shortcuts and AltGraph text', () => {
  class Key {
    constructor(readonly value: string, readonly altGraph = false) {}
    get key() { return this.value }
    get code() { return `Key${this.value.toUpperCase()}` }
    get keyCode() { return this.value.toUpperCase().charCodeAt(0) }
    get repeat() { return false }
    get location() { return 0 }
    get altKey() { return this.altGraph }
    get ctrlKey() { return this.altGraph }
    get metaKey() { return !this.altGraph }
    get shiftKey() { return false }
    getModifierState(name: string) { return name === 'AltGraph' && this.altGraph }
  }
  const select = keyMessage(TAB.id, 'down', new Key('a'))
  expect(select).toMatchObject({ type: 'key', modifiers: 4, altGraph: false })
  expect(select).not.toHaveProperty('text')
  expect(localKey(new Key('v'))).toBe(true)
  expect(localKey(new Key('v', true))).toBe(false)
  expect(keyMessage(TAB.id, 'down', new Key('v', true)))
    .toMatchObject({ text: 'v', modifiers: 3, altGraph: true })
})

/** A session over a stream the test drives, with a clock it controls. */
function session(options: Partial<LiveSessionOptions> = {}) {
  const sent: LiveViewerMessage[] = []
  const pictures: Array<[string, number, number]> = []
  const decoder = new TextDecoder()
  let handlers: UserStreamHandlers | null = null
  let now = 0
  const sink: PictureSink = {
    start: (stream) => pictures.push(['start', stream.generation, stream.width] as never) as never,
    show: (frame) => pictures.push(['show', frame.generation, frame.sequence] as never) as never,
    stop: () => pictures.push(['stop', 0, 0] as never) as never,
  }
  const live = new LiveSession({
    open: (given: UserStreamHandlers) => {
      handlers = given
      return {
        send: (bytes) => {
          if (bytes[4] === 1) {
            sent.push(JSON.parse(decoder.decode(bytes.subarray(5))) as LiveViewerMessage)
          }
        },
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
    pictures,
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

test('a view says hello, learns the tabs and acknowledges what it shows', () => {
  const view = session()
  expect(view.sent[0]).toEqual({ type: 'hello', platform: 'mac' })
  view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
  expect(view.live.state).toMatchObject({ connection: 'live', running: true, watched: TAB.id })
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200, viewport: WEB, scale: 1 }))
  view.receive(video(TAB.id, 1, 4, true, [0, 0, 0, 1]))
  // Frames of an older generation are not shown.
  view.receive(video(TAB.id, 0, 5, true, [0, 0, 0, 1]))
  expect(view.pictures).toEqual([['start', 1, 1600], ['show', 1, 4]] as never)
  view.live.showed(1, 4, 0)
  expect(view.sent.at(-1)).toEqual({ type: 'ack', generation: 1, sequence: 4, decodeQueue: 0 })
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

test('a stalled stream discards input and resumes from a key frame', () => {
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
  view.live.input(pointerMessage(TAB.id, 'down', { x: 1, y: 2 }, { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, button: 0, buttons: 1 }))
  expect(view.sent).toHaveLength(before)
  view.receive(moduleFrame({ type: 'heartbeat' }))
  expect(view.live.state.connection).toBe('live')
  expect(view.sent.at(-1)).toEqual({ type: 'keyframe', generation: 1 })
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
    expect(ended).toEqual([SILENT_STREAM])
    expect(view.live.state).toMatchObject({ connection: 'opening', ended: SILENT_STREAM })
    jest.advanceTimersByTime(1_000)
    expect(hellos()).toBe(2)
    view.receive(moduleFrame({ type: 'state', running: true, list: 1, tabs: [TAB], watched: TAB.id }))
    // The laptop sleeps for 80 seconds with the page shown: the clock goes
    // on, the watch's timer does not, and the page comes back online.
    jest.setSystemTime(Date.now() + 80_000)
    pageReturned()
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
  view.receive(moduleFrame({ type: 'notice', code: 'input_failed', message: 'the page went away' }))
  view.receive(moduleFrame({ type: 'notice', code: 'timeout', message: 'screen: timed out' }))
  view.receive(video(TAB.id, 1, 2, false, [0, 0, 0, 1]))
  expect(view.live.state.pictureless).toBeNull()
  expect(told).toEqual(['input_failed', 'timeout'])
  // A Host that cannot capture says so for each tab the page watches.
  view.receive(moduleFrame({ type: 'notice', code: 'capture_unavailable', message: 'this CPU reports SME without SVE' }))
  expect(view.live.state.pictureless).toBe('capture_unavailable')
  view.live.watch(null)
  expect(view.live.state.pictureless).toBeNull()
  expect(told).toEqual(['input_failed', 'timeout'])
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
  view.live.input(pointerMessage(TAB.id, 'move', { x: 10, y: 20 }, Object.assign({}, event)))
  expect(view.sent).toHaveLength(before)
  view.live.input(pointerMessage(TAB.id, 'move', { x: 10, y: 20 }, event))
  expect(view.sent.at(-1)).toMatchObject({ type: 'pointer', action: 'move', buttons: 0, clickCount: 0 })
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

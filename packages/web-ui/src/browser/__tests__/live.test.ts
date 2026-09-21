import { expect, test } from 'bun:test'
import type { LiveControl, LiveModuleMessage, LiveTab, LiveViewerMessage } from '@demicodes/browser-protocol/live'
import { LiveFrameReader, encodeFile, encodeMessage, type LiveFrame } from '../frames'
import { keyMessage, localKey, modifiers, pointerMessage, viewerPlatform, wheelMessage } from '../input'
import { LiveSession, type LiveSessionOptions, type LiveStreamHandlers, type PictureSink } from '../session'
import { panelSize, placePicture, panelRect, tabPoint, viewportChoices } from '../view'

const WEB = { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' } as const
const PHONE = { width: 390, height: 844, devicePixelRatio: 2, mode: 'mobile' } as const

/** A module frame as the Host writes it. */
function video(tab: string, generation: number, sequence: number, key: boolean, data: number[]): Uint8Array {
  const payload = new Uint8Array(48 + data.length)
  const header = new DataView(payload.buffer)
  payload.set(new TextEncoder().encode(tab))
  header.setUint32(24, generation)
  header.setUint32(28, sequence)
  header.setUint8(32, key ? 1 : 0)
  header.setFloat64(36, 1234)
  header.setUint16(44, 1600)
  header.setUint16(46, 1200)
  payload.set(data, 48)
  const frame = new Uint8Array(5 + payload.length)
  new DataView(frame.buffer).setUint32(0, 1 + payload.length)
  frame[4] = 2
  frame.set(payload, 5)
  return frame
}

function moduleFrame(message: LiveModuleMessage): Uint8Array {
  const json = new TextEncoder().encode(JSON.stringify(message))
  const frame = new Uint8Array(5 + json.length)
  new DataView(frame.buffer).setUint32(0, 1 + json.length)
  frame[4] = 1
  frame.set(json, 5)
  return frame
}

test('frames split across chunks and join within one', () => {
  const reader = new LiveFrameReader()
  const bytes = new Uint8Array([
    ...moduleFrame({ type: 'heartbeat' }),
    ...video('t_aaaaaaaaaaaaaaaaaaaaaa', 3, 7, true, [0, 0, 0, 1, 9]),
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
    expect(frame).toMatchObject({ tab: 't_aaaaaaaaaaaaaaaaaaaaaa', generation: 3, sequence: 7, key: true, width: 1600, height: 1200 })
    expect([...frame!.data]).toEqual([0, 0, 0, 1, 9])
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

test('a web tab fills the panel and a phone keeps its size in the middle', () => {
  const panel = panelSize(800.4, 600.6)
  expect(panel).toEqual({ width: 800, height: 601 })
  const web = placePicture(WEB, { width: 800, height: 600 })
  expect(web).toMatchObject({ scale: 1, left: 0, top: 0, width: 800, height: 600 })
  const phone = placePicture(PHONE, { width: 800, height: 600 })
  expect(phone.scale).toBeCloseTo(600 / 844)
  expect(phone.left).toBeCloseTo((800 - 390 * phone.scale) / 2)
  expect(phone.top).toBe(0)
})

test('panel points map to the tab and back, within the picture', () => {
  const placement = placePicture(PHONE, { width: 800, height: 600 })
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

test('the viewer platform comes from its own browser', () => {
  expect(viewerPlatform({ platform: 'MacIntel', userAgent: 'Mozilla/5.0 (Macintosh)' })).toBe('mac')
  expect(viewerPlatform({ platform: 'Win32', userAgent: 'Mozilla/5.0 (Windows NT 10.0)' })).toBe('windows')
  expect(viewerPlatform({ platform: '', userAgent: 'Mozilla/5.0 (X11; Linux x86_64)' })).toBe('linux')
  expect(viewerPlatform({ platform: '', userAgent: 'Mozilla/5.0 (SomethingElse)' })).toBe('other')
})

test('pointer, wheel and key events carry what the page needs', () => {
  const none = { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false }
  const down = pointerMessage('t_a', 'down', { x: 5, y: 6 }, { ...none, button: 2, buttons: 2, detail: 1 })
  expect(down).toMatchObject({ type: 'pointer', action: 'down', button: 'right', buttons: 2, clickCount: 1, modifiers: 0 })
  const move = pointerMessage('t_a', 'move', { x: 1, y: 2 }, { ...none, button: 0, buttons: 1, detail: 0 })
  expect(move).toMatchObject({ button: 'left', clickCount: 0 })
  const lines = wheelMessage('t_a', { x: 0, y: 0 }, { ...none, deltaX: 0, deltaY: 3, deltaMode: 1 }, 600)
  expect(lines).toMatchObject({ deltaY: 48 })
  const pages = wheelMessage('t_a', { x: 0, y: 0 }, { ...none, deltaX: 0, deltaY: -1, deltaMode: 2 }, 600)
  expect(pages).toMatchObject({ deltaY: -600 })
  const typed = keyMessage('t_a', 'down', { ...none, key: 'a', code: 'KeyA', keyCode: 65, repeat: false, location: 0, altGraph: false })
  expect(typed).toMatchObject({ type: 'key', action: 'down', text: 'a', modifiers: 0 })
  const shortcut = keyMessage('t_a', 'down', { ...none, ctrlKey: true, key: 'c', code: 'KeyC', keyCode: 67, repeat: false, location: 0, altGraph: false })
  expect(shortcut).not.toHaveProperty('text')
  expect(shortcut.type === 'key' && shortcut.modifiers).toBe(2)
  const released = keyMessage('t_a', 'up', { ...none, key: 'a', code: 'KeyA', keyCode: 65, repeat: true, location: 0, altGraph: false })
  expect(released).toMatchObject({ action: 'up', repeat: false })
  expect(modifiers({ altKey: true, ctrlKey: false, metaKey: true, shiftKey: true })).toBe(1 | 4 | 8)
  // The viewer's own paste reaches the page as a paste event.
  expect(localKey({ ...none, metaKey: true, key: 'v', code: 'KeyV', keyCode: 86, repeat: false, location: 0, altGraph: false })).toBe(true)
  expect(localKey({ ...none, metaKey: true, key: 'c', code: 'KeyC', keyCode: 67, repeat: false, location: 0, altGraph: false })).toBe(false)
})

/** A session over a stream the test drives, with a clock it controls. */
function session(options: Partial<LiveSessionOptions> = {}) {
  const sent: LiveViewerMessage[] = []
  const pictures: Array<[string, number, number]> = []
  const decoder = new TextDecoder()
  let handlers: LiveStreamHandlers | null = null
  let now = 0
  const sink: PictureSink = {
    start: (generation, width, height) => pictures.push(['start', generation, width] as never) as never,
    show: (frame) => pictures.push(['show', frame.generation, frame.sequence] as never) as never,
    stop: () => pictures.push(['stop', 0, 0] as never) as never,
  }
  const live = new LiveSession({
    open: (given: LiveStreamHandlers) => {
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
    now: () => now,
    reconnect: () => null,
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
  id: 't_aaaaaaaaaaaaaaaaaaaaaa',
  title: 'Example',
  url: 'https://example.test/',
  createdBy: { kind: 'agent', nodeId: 'node' },
  viewport: { ...WEB },
}

test('a view says hello, learns the tabs and acknowledges what it shows', () => {
  const view = session()
  expect(view.sent[0]).toEqual({ type: 'hello', platform: 'mac' })
  view.receive(moduleFrame({ type: 'state', running: true, tabs: [TAB], watched: TAB.id }))
  expect(view.live.state).toMatchObject({ connection: 'live', running: true, watched: TAB.id })
  expect(view.live.viewport).toEqual(WEB)
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200 }))
  view.receive(video(TAB.id, 1, 4, true, [0, 0, 0, 1]))
  // Frames of an older generation are not shown.
  view.receive(video(TAB.id, 0, 5, true, [0, 0, 0, 1]))
  expect(view.pictures).toEqual([['start', 1, 1600], ['show', 1, 4]] as never)
  view.live.showed(1, 4, 0)
  expect(view.sent.at(-1)).toEqual({ type: 'ack', generation: 1, sequence: 4, decodeQueue: 0 })
})

test('a stalled stream discards input and resumes from a key frame', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, tabs: [TAB], watched: TAB.id }))
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200 }))
  view.advance(300)
  view.live.tick()
  expect(view.live.state.connection).toBe('live')
  view.advance(1200)
  view.live.tick()
  expect(view.live.state.connection).toBe('stalled')
  expect(view.sent.at(-1)).toEqual({ type: 'release' })
  const before = view.sent.length
  view.live.input(pointerMessage(TAB.id, 'down', { x: 1, y: 2 }, { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, button: 0, buttons: 1, detail: 1 }))
  expect(view.sent).toHaveLength(before)
  view.receive(moduleFrame({ type: 'heartbeat' }))
  expect(view.live.state.connection).toBe('live')
  expect(view.sent.at(-1)).toEqual({ type: 'keyframe', generation: 1 })
})

test('operating is activity, at most every thirty seconds, and moving is not', () => {
  const operations: number[] = []
  const view = session({ onOperation: () => operations.push(operations.length) })
  view.receive(moduleFrame({ type: 'state', running: true, tabs: [TAB], watched: TAB.id }))
  const none = { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false }
  view.live.input(pointerMessage(TAB.id, 'move', { x: 1, y: 1 }, { ...none, button: 0, buttons: 0, detail: 0 }))
  expect(operations).toHaveLength(0)
  view.live.input(pointerMessage(TAB.id, 'down', { x: 1, y: 1 }, { ...none, button: 0, buttons: 1, detail: 1 }))
  view.advance(10_000)
  view.live.input(keyMessage(TAB.id, 'down', { ...none, key: 'a', code: 'KeyA', keyCode: 65, repeat: false, location: 0, altGraph: false }))
  expect(operations).toHaveLength(1)
  view.advance(21_000)
  view.live.input(keyMessage(TAB.id, 'down', { ...none, key: 'b', code: 'KeyB', keyCode: 66, repeat: false, location: 0, altGraph: false }))
  expect(operations).toHaveLength(2)
})

test('a choice names the revision the viewer saw, and a dialog is answered once', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, tabs: [TAB], watched: TAB.id }))
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
})

test('the view ends with the reason the module or the backend gave', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'ended', reason: 'browser_ended' }))
  expect(view.live.state).toMatchObject({ connection: 'ended', ended: 'browser_ended' })
  expect(view.pictures.at(-1)).toEqual(['stop', 0, 0] as never)
})

test('a view that ends opens again, watching what the viewer watched', async () => {
  const view = session({ reconnect: () => 0 })
  view.live.panel({ width: 800, height: 600 }, 2, { width: 1440, height: 900 })
  view.receive(moduleFrame({ type: 'state', running: true, tabs: [TAB], watched: TAB.id }))
  view.close('host_unreachable')
  expect(view.live.state).toMatchObject({ connection: 'opening', running: false })
  expect(view.live.state.tabs).toHaveLength(0)
  await Bun.sleep(5)
  const opened = view.sent.slice(-3)
  expect(opened).toEqual([
    { type: 'hello', platform: 'mac' },
    { type: 'panel', width: 800, height: 600, devicePixelRatio: 2, screenWidth: 1440, screenHeight: 900 },
    { type: 'watch', tab: TAB.id },
  ])
  // The page stops asking once the view itself is closed.
  view.live.close()
  view.close('closed')
  await Bun.sleep(5)
  expect(view.sent.at(-1)).toEqual({ type: 'watch', tab: TAB.id })
  expect(view.live.state.connection).toBe('ended')
})

test('a view the module ended and the socket then closed opens again once', async () => {
  const view = session({ reconnect: () => 0 })
  view.receive(moduleFrame({ type: 'ended', reason: 'browser_ended' }))
  view.close('closed')
  await Bun.sleep(5)
  expect(view.sent.filter((message) => message.type === 'hello')).toHaveLength(2)
  view.live.close()
})

test('a picture ends the notice that capture failed, and no other', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, tabs: [TAB], watched: TAB.id }))
  view.receive(moduleFrame({ type: 'stream', tab: TAB.id, generation: 1, width: 1600, height: 1200 }))
  view.receive(moduleFrame({ type: 'notice', code: 'capture_failed', message: 'the capture extension did not connect' }))
  expect(view.live.state.notice?.code).toBe('capture_failed')
  view.receive(video(TAB.id, 1, 1, true, [0, 0, 0, 1]))
  expect(view.live.state.notice).toBeNull()
  view.receive(moduleFrame({ type: 'notice', code: 'input_failed', message: 'the page went away' }))
  view.receive(video(TAB.id, 1, 2, false, [0, 0, 0, 1]))
  expect(view.live.state.notice?.code).toBe('input_failed')
})

test('a message the protocol refuses is never sent, so the module does not end the view', () => {
  const view = session()
  view.receive(moduleFrame({ type: 'state', running: true, tabs: [TAB], watched: TAB.id }))
  // A DOM event's fields are getters on its prototype: a spread copy of one has none of them.
  class Hover {
    get button() { return 0 }
    get buttons() { return 0 }
    get detail() { return 0 }
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

test('text the watched tab copies reaches the viewer', () => {
  const copied: string[] = []
  const view = session({ onClipboard: (text) => copied.push(text) })
  view.receive(moduleFrame({ type: 'clipboard', text: 'copy me' }))
  expect(copied).toEqual(['copy me'])
})

import { expect, test } from 'bun:test'
import { LIVE_CHUNK_DATA_BYTES } from '@demicodes/protocol'
import { LiveFrameAssembler, LiveFrameReader, encodeControl, encodeVideo, framed, type LiveRawFrame, type LiveVideoChunk } from '../frames'
import { ClickCount, keyInput, modifiers, pointerInput, touchInput, wheelInput } from '../input'

/** A picture as a Host frames it. */
function video(source: string, generation: number, sequence: number, key: boolean, data: number[]): Uint8Array {
  return encodeVideo({ source, generation, sequence, key, timestamp: 1234, width: 1600, height: 1200, data: new Uint8Array(data) })
}

test('frames split across chunks and join within one', () => {
  const reader = new LiveFrameReader()
  const bytes = new Uint8Array([
    ...encodeControl({ type: 'heartbeat' }),
    ...video('m1', 3, 7, true, [0, 0, 0, 1, 9]),
  ])
  const oneByOne: LiveRawFrame[] = []
  for (const byte of bytes) {
    oneByOne.push(...reader.read(new Uint8Array([byte])))
  }
  const together = new LiveFrameReader().read(bytes)
  for (const frames of [oneByOne, together]) {
    expect(frames).toHaveLength(2)
    expect(frames[0]!.kind === 'control' && new TextDecoder().decode(frames[0]!.payload)).toBe('{"type":"heartbeat"}')
    const frame = frames[1]!.kind === 'video' ? frames[1]!.frame : null
    expect(frame).toMatchObject({ source: 'm1', generation: 3, sequence: 7, key: true, width: 1600, height: 1200 })
    expect([...frame!.data]).toEqual([0, 0, 0, 1, 9])
  }
})

/** The chunks of a picture whose data is `length` bytes counting up, as the page reads them. */
function chunks(sequence: number, key: boolean, length: number): LiveVideoChunk[] {
  const data = Array.from({ length }, (_, index) => index % 251)
  return new LiveFrameReader()
    .read(video('m1', 1, sequence, key, data))
    .map((frame) => (frame.kind === 'video' ? frame.frame : expect.unreachable()))
}

/**
 * A direct path delivers chunks in any order: the page joins each frame whole and
 * hands frames on in sequence, a frame waiting for the one before it, and a key
 * frame for none.
 *
 * Planted defects this catches:
 *
 * - Joining chunks in arrival order.
 * - Handing a frame on before the one before it.
 * - Waiting before a key frame.
 */
test('chunks that come out of order make whole frames, handed on in sequence', () => {
  const first = chunks(1, true, 2 * LIVE_CHUNK_DATA_BYTES + 10)
  const second = chunks(2, false, LIVE_CHUNK_DATA_BYTES + 1)
  const third = chunks(3, false, 5)
  expect([first.length, second.length, third.length]).toEqual([3, 2, 1])
  const assembler = new LiveFrameAssembler()
  const order = [first[2]!, second[1]!, first[0]!, third[0]!, second[0]!]
  const handed = order.map((chunk) => assembler.add(chunk).map((frame) => frame.sequence))
  expect(handed).toEqual([[], [], [], [], []])
  // The last chunk of the first frame lets all three go, in order and whole.
  const released = assembler.add(first[1]!)
  expect(released.map((frame) => frame.sequence)).toEqual([1, 2, 3])
  expect([...released[0]!.data]).toEqual(Array.from({ length: 2 * LIVE_CHUNK_DATA_BYTES + 10 }, (_, index) => index % 251))
  expect(released[1]!.data.length).toBe(LIVE_CHUNK_DATA_BYTES + 1)
  // A key frame goes on without the frames before it, which no longer matter.
  const late = chunks(4, false, 5)
  expect(assembler.add(chunks(6, true, 5)[0]!).map((frame) => frame.sequence)).toEqual([6])
  expect(assembler.add(late[0]!)).toEqual([])
})

test('a frame the protocol refuses is an error, never a frame', () => {
  const refused: Array<[Uint8Array, string]> = [
    [framed(3, new Uint8Array(8)), 'kind 3'],
    [framed(2, new Uint8Array([0, 0, 0, 1])), 'shorter than its header'],
    [(() => { const bytes = video('m1', 1, 1, true, [1]); bytes[5 + 25] = 1; return bytes })(), 'outside its frame'],
    [new Uint8Array([0, 0, 0, 0]), 'impossible size'],
  ]
  for (const [bytes, reason] of refused) {
    expect(() => new LiveFrameReader().read(bytes)).toThrow(reason)
  }
})

const NONE = { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false }

test('pointer, wheel, key and touch input carry what a source needs', () => {
  const down = pointerInput('down', { x: 5, y: 6 }, { ...NONE, button: 2, buttons: 2 })
  expect(down).toMatchObject({ type: 'pointer', action: 'down', button: 'right', buttons: 2, clickCount: 1, modifiers: 0 })
  const move = pointerInput('move', { x: 1, y: 2 }, { ...NONE, button: 0, buttons: 1 })
  expect(move).toMatchObject({ button: 'left', clickCount: 0 })
  const lines = wheelInput({ x: 0, y: 0 }, { ...NONE, deltaX: 0, deltaY: 3, deltaMode: 1 }, 600)
  expect(lines).toMatchObject({ deltaY: 48 })
  const pages = wheelInput({ x: 0, y: 0 }, { ...NONE, deltaX: 0, deltaY: -1, deltaMode: 2 }, 600)
  expect(pages).toMatchObject({ deltaY: -600 })
  const typed = keyInput('down', { ...NONE, key: 'a', code: 'KeyA', keyCode: 65, repeat: false, location: 0, getModifierState: () => false })
  expect(typed).toMatchObject({ type: 'key', action: 'down', text: 'a', modifiers: 0 })
  const shortcut = keyInput('down', { ...NONE, ctrlKey: true, key: 'c', code: 'KeyC', keyCode: 67, repeat: false, location: 0, getModifierState: () => false })
  expect(shortcut).not.toHaveProperty('text')
  expect(shortcut.type === 'key' && shortcut.modifiers).toBe(2)
  const released = keyInput('up', { ...NONE, key: 'a', code: 'KeyA', keyCode: 65, repeat: true, location: 0, getModifierState: () => false })
  expect(released).toMatchObject({ action: 'up', repeat: false })
  expect(modifiers({ altKey: true, ctrlKey: false, metaKey: true, shiftKey: true })).toBe(1 | 4 | 8)
  // Two fingers that pinch, and a cancel that lets go of every finger.
  const fingers = [{ id: 1, x: 10, y: 20 }, { id: 2, x: 30, y: 40 }]
  expect(touchInput('start', fingers)).toEqual({ type: 'touch', action: 'start', contacts: fingers })
  expect(touchInput('cancel', fingers)).toEqual({ type: 'touch', action: 'cancel', contacts: [] })
})

test('presses close in time and place count as one double or triple click, as a page needs them', () => {
  // A pointer event's own detail is 0, so the view counts.
  const clicks = new ClickCount()
  const press = (timeStamp: number, clientX = 10, button = 0) => clicks.press({ timeStamp, clientX, clientY: 10, button })
  expect([press(0), press(200), press(400), press(600)]).toEqual([1, 2, 3, 3])
  expect(press(2000)).toBe(1)
  expect(press(2100, 30)).toBe(1)
  expect(press(2200, 30, 2)).toBe(1)
  expect(clicks.current).toBe(1)
  expect(pointerInput('down', { x: 1, y: 1 }, { ...NONE, button: 0, buttons: 1 }, 2)).toMatchObject({ clickCount: 2 })
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
  const select = keyInput('down', new Key('a'))
  expect(select).toMatchObject({ type: 'key', modifiers: 4, altGraph: false })
  expect(select).not.toHaveProperty('text')
  expect(keyInput('down', new Key('v', true))).toMatchObject({ text: 'v', modifiers: 3, altGraph: true })
})

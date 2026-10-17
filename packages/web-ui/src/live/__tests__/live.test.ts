import { expect, test } from 'bun:test'
import { LiveFrameReader, encodeControl, encodeVideo, framed, type LiveRawFrame } from '../frames'
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

test('a frame the protocol refuses is an error, never a frame', () => {
  const refused: Array<[Uint8Array, string]> = [
    [framed(3, new Uint8Array(8)), 'kind 3'],
    [framed(2, new Uint8Array([0, 0, 0, 1])), 'shorter than its header'],
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

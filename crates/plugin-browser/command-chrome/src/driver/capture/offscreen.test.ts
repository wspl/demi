import { expect, test } from 'bun:test'
import { readFile } from 'node:fs/promises'
import { setImmediate } from 'node:timers/promises'
import { runInNewContext } from 'node:vm'

// Runs the capture extension's offscreen document, as `launch.rs` ships it,
// with controlled browser APIs instead of Chrome. `bun run test` runs it.

/** The extension running against controlled APIs, and what it did to them. */
async function load(scenario: string) {
  const source = await readFile(new URL('./offscreen.js', import.meta.url), 'utf8')
  const listeners = new Map<string, (event: { data: string }) => void>()
  const timers = new Map<number, { callback: () => void; delay: number }>()
  const grants: string[] = []
  const stopped: string[] = []
  const sent: string[] = []
  const released: string[] = []
  let timerId = 0
  let socket: Socket | undefined
  let releaseSecond: () => void = () => { throw new Error('second grant has not started') }
  const secondGrant = new Promise<void>(resolve => { releaseSecond = resolve })
  const readers = new Map<string, () => void>()
  class Socket {
    static OPEN = 1
    constructor() { socket = this }
    readyState = 1
    bufferedAmount = 0
    addEventListener(name: string, listener: (event: { data: string }) => void): void {
      listeners.set(name, listener)
    }
    send(message: string): void {
      sent.push(JSON.parse(message).type)
    }
    close(): void {
      this.readyState = 3
      listeners.get('close')?.({ data: '' })
    }
  }
  class Processor {
    readable: { getReader: () => { read: () => Promise<{ done: true }>; cancel: () => Promise<void> } }
    constructor({ track }: { track: { id: string } }) {
      let end: () => void = () => {}
      const ended = new Promise<{ done: true }>(resolve => { end = () => resolve({ done: true }) })
      readers.set(track.id, end)
      this.readable = { getReader: () => ({ read: () => ended, cancel: async () => { end() } }) }
    }
  }
  runInNewContext(source.replace("import { socket as address, codec } from './config.js';", "const address = 'ws://capture.test'; const codec = 'test-codec';"), {
    WebSocket: Socket,
    MediaStreamTrackProcessor: Processor,
    VideoEncoder: { isConfigSupported: async () => ({ supported: true }) },
    chrome: { runtime: { sendMessage: async (message: { type: string; target: string; tabId: string }) => {
      if (message.type === 'released') released.push(message.tabId)
      if (message.type !== 'grant') return {}
      grants.push(message.target)
      if (message.target === 'second') await secondGrant
      return { tabId: message.target, streamId: message.target }
    } } },
    navigator: { mediaDevices: { getUserMedia: async (options: { video: { mandatory: { chromeMediaSourceId: string } } }) => {
      if (scenario === 'aborted') throw Object.assign(new Error('Failed due to shutdown'), { name: 'AbortError' })
      const id = options.video.mandatory.chromeMediaSourceId
      const track = { id, stop: () => { stopped.push(id) }, applyConstraints: async () => {} }
      return { getVideoTracks: () => [track], getTracks: () => [track] }
    } } },
    setInterval: () => ++timerId,
    clearInterval: () => {},
    setTimeout: (callback: () => void, delay: number) => {
      const id = ++timerId
      timers.set(id, { callback, delay })
      return id
    },
    clearTimeout: (id: number) => { timers.delete(id) },
    performance,
    console,
  })
  const message = listeners.get('message')
  const close = listeners.get('close')
  expect(message).toBeDefined()
  expect(close).toBeDefined()
  if (!message || !close) throw new Error('extension did not register socket handlers')
  return {
    message,
    close,
    timers,
    grants,
    stopped,
    sent,
    released,
    releaseSecond: () => releaseSecond(),
    socket: () => socket,
    /** Ends every reader, as the tracks do once stopped. */
    async finish() {
      releaseSecond()
      close({ data: '' })
      for (const end of readers.values()) end()
      await setImmediate()
    },
  }
}

for (const scenario of ['stopped', 'silent', 'aborted', 'disconnected']) {
  test(`the ${scenario} capture leaves replacement to the Host`, async () => {
    const { message, timers, grants, stopped, sent, releaseSecond, socket, finish } = await load(scenario)
    try {
      message({ data: JSON.stringify({ type: 'start', capture: 1, target: 'first', width: 800, height: 600, sourceWidth: 800, sourceHeight: 600, fps: 60, bitrate: 1000000 }) })
      await setImmediate()
      if (scenario === 'aborted') {
        // The Host recreates the extension; the capture fails meanwhile.
        expect(sent).toEqual(['recreate', 'error'])
        expect(grants).toEqual(['first'])
        expect(stopped).toEqual([])
        return
      }
      message({ data: JSON.stringify({ type: 'start', capture: 2, target: 'second', width: 800, height: 600, sourceWidth: 800, sourceHeight: 600, fps: 60, bitrate: 1000000 }) })
      await setImmediate()
      if (scenario === 'disconnected') {
        const open = socket()
        if (!open) throw new Error('extension did not open its socket')
        open.close()
        releaseSecond()
        await setImmediate()
        expect(stopped).toEqual(['first', 'second'])
        expect(grants).toEqual(['first', 'second'])
        return
      }
      // Recovery becomes due while another start holds the queue and stop is pending.
      if (scenario === 'stopped') {
        message({ data: JSON.stringify({ type: 'stop', capture: 1 }) })
      }
      for (const delay of [1500, 2000]) {
        const timer = [...timers.values()].find(timer => timer.delay === delay)
        if (!timer) throw new Error(`no ${delay}ms capture watchdog`)
        timer.callback()
      }
      releaseSecond()
      await setImmediate()
      expect(stopped).toContain('first')
      expect(grants).toEqual(['first', 'second'])
    } finally {
      await finish()
    }
  })
}

// Chrome grants a tab one capture, so a recording and a live view of the same
// tab read one source: the second consumer asks for no second grant, either
// one stopping leaves the capture to the other, and the last releases the tab.
test('a recording and a live view of one tab share its capture', async () => {
  const { message, grants, stopped, released, sent, finish } = await load('shared')
  try {
    message({ data: JSON.stringify({ type: 'start', capture: 1, target: 'tab', width: 800, height: 600, sourceWidth: 800, sourceHeight: 600, fps: 30, bitrate: 1000000 }) })
    await setImmediate()
    message({ data: JSON.stringify({ type: 'record', capture: 2, target: 'tab', width: 800, height: 600, keyInterval: 120 }) })
    await setImmediate()
    expect(grants).toEqual(['tab'])
    expect(sent).toEqual(['started', 'started'])
    // The live view leaves; the recording keeps the capture.
    message({ data: JSON.stringify({ type: 'stop', capture: 1 }) })
    await setImmediate()
    expect(stopped).toEqual([])
    expect(sent).toEqual(['started', 'started', 'stopped'])
    // The recording leaves; the tab is released once.
    message({ data: JSON.stringify({ type: 'stop', capture: 2 }) })
    await setImmediate()
    expect(stopped).toEqual(['tab'])
    expect(released).toEqual(['tab'])
    expect(sent).toEqual(['started', 'started', 'stopped', 'stopped'])
  } finally {
    await finish()
  }
})

/**
 * A frame the stubbed tab capture reads: NV12, 100 × 80, every byte `shade`
 * except one luma byte at `mark` when there is one.
 */
class StubFrame {
  displayWidth = 100
  displayHeight = 80
  visibleRect = { x: 0, y: 0, width: 100, height: 80 }
  format = 'NV12'
  colorSpace = { matrix: 'smpte170m', fullRange: false }
  constructor(readonly timestamp: number, readonly shade = timestamp % 251, readonly mark?: [number, number]) {}
  clone(): StubFrame { return new StubFrame(this.timestamp, this.shade, this.mark) }
  allocationSize(): number { return 100 * 80 * 3 / 2 }
  async copyTo(bytes: Uint8Array): Promise<Array<{ offset: number; stride: number }>> {
    bytes.fill(this.shade)
    if (this.mark) bytes[this.mark[1] * 100 + this.mark[0]] = this.shade ^ 0xff
    return [{ offset: 0, stride: 100 }, { offset: 8000, stride: 100 }]
  }
  close(): void {}
}

/**
 * The extension with a tab whose frames the test hands over, and a video
 * encoder that keeps every frame until the test, or a flush, makes it give
 * its picture.
 */
async function loadRecorder() {
  const source = await readFile(new URL('./offscreen.js', import.meta.url), 'utf8')
  const listeners = new Map<string, (event: { data: string }) => void>()
  /** What the extension sent, in order: its events by type, its pictures as `picture`. */
  const sent: Array<string | { missed: number }> = []
  /** The pictures the extension sent, as it packed them. */
  const pictures: ArrayBuffer[] = []
  const opened: Array<{ maxFrameRate: number; width: number; height: number }> = []
  /** The frames the browser discarded before the extension read them, as the track counts. */
  const stats = { discardedFrames: 0 }
  const queue: Array<(value: { value?: StubFrame; done?: true }) => void> = []
  const waiting: Array<{ value?: StubFrame; done?: true }> = []
  let encoder: StubEncoder | undefined
  class StubEncoder {
    state = 'unconfigured'
    pending: number[] = []
    /** Whether each frame given was asked to be a key frame. */
    keys: boolean[] = []
    constructor(private readonly init: { output: (chunk: unknown) => void }) { encoder = this }
    get encodeQueueSize(): number { return this.pending.length }
    configure(): void { this.state = 'configured' }
    encode(frame: StubFrame, options?: { keyFrame?: boolean }): void {
      this.pending.push(frame.timestamp)
      this.keys.push(options?.keyFrame === true)
    }
    /** Gives the pictures of `count` frames, the oldest first. */
    emit(count: number): void {
      for (const timestamp of this.pending.splice(0, count)) {
        this.init.output({ type: 'delta', timestamp, byteLength: 1, copyTo: () => {} })
      }
    }
    async flush(): Promise<void> { this.emit(this.pending.length) }
    close(): void { this.state = 'closed' }
  }
  class Socket {
    static OPEN = 1
    readyState = 1
    bufferedAmount = 0
    addEventListener(name: string, listener: (event: { data: string }) => void): void { listeners.set(name, listener) }
    send(message: string | ArrayBuffer): void {
      if (typeof message === 'string') {
        const event = JSON.parse(message)
        sent.push(event.type === 'missed' ? { missed: event.count } : event.type)
      } else {
        sent.push('picture')
        pictures.push(message)
      }
    }
  }
  class Processor {
    readable = {
      getReader: () => ({
        read: () => new Promise<{ value?: StubFrame; done?: true }>(resolve => {
          const next = waiting.shift()
          if (next) resolve(next)
          else queue.push(resolve)
        }),
        cancel: async () => { for (const resolve of queue.splice(0)) resolve({ done: true }) },
      }),
    }
  }
  runInNewContext(source.replace("import { socket as address, codec } from './config.js';", "const address = 'ws://capture.test'; const codec = 'test-codec';"), {
    WebSocket: Socket,
    MediaStreamTrackProcessor: Processor,
    VideoFrame: class extends StubFrame { constructor(from: StubFrame, init: { timestamp: number }) { super(init.timestamp ?? from.timestamp, from.shade, from.mark) } },
    VideoEncoder: Object.assign(StubEncoder, { isConfigSupported: async () => ({ supported: true }) }),
    chrome: { runtime: { sendMessage: async (message: { type: string; target: string }) => message.type === 'grant' ? { tabId: message.target, streamId: message.target } : {} } },
    navigator: { mediaDevices: { getUserMedia: async (options: { video: { mandatory: { maxFrameRate: number; minWidth: number; minHeight: number } } }) => {
      const { maxFrameRate, minWidth, minHeight } = options.video.mandatory
      opened.push({ maxFrameRate, width: minWidth, height: minHeight })
      const track = { stop: () => {}, contentHint: '', stats }
      return { getVideoTracks: () => [track], getTracks: () => [track] }
    } } },
    setInterval: () => 1,
    clearInterval: () => {},
    setTimeout: () => 1,
    clearTimeout: () => {},
    performance,
    console,
  })
  const message = listeners.get('message')
  if (!message) throw new Error('extension did not register socket handlers')
  return {
    send: (command: object) => message({ data: JSON.stringify(command) }),
    sent,
    pictures,
    opened,
    stats,
    encoder: () => encoder,
    /** The tab draws a frame. */
    draw(timestamp: number, shade?: number, mark?: [number, number]): void {
      const frame = { value: new StubFrame(timestamp, shade, mark) }
      const next = queue.shift()
      if (next) next(frame)
      else waiting.push(frame)
    },
  }
}

// Planted defect this catches: a stop that retires the recording before its
// encoder's flush, which drops the last pictures, the ones a model looks for.
test('the pictures a stop makes the encoder flush are delivered before it is acknowledged', async () => {
  const extension = await loadRecorder()
  extension.send({ type: 'record', capture: 1, target: 'tab', width: 100, height: 80, keyInterval: 120 })
  await setImmediate()
  for (const at of [1000, 17000, 33000]) extension.draw(at)
  await setImmediate()
  expect(extension.encoder()?.pending.length).toBe(3)
  extension.send({ type: 'stop', capture: 1 })
  await setImmediate()
  await setImmediate()
  expect(extension.sent).toEqual(['started', 'picture', 'picture', 'picture', 'stopped'])
})

// Planted defects this catches: a capture opened at the live view's pace,
// which a recording that joins cannot raise, and one capped at 60 frames a
// second, which skips every second frame of a 120 Hz screen without a count.
test('a tab is captured at the full frame rate, and at its own size, whichever consumer opens it', async () => {
  const extension = await loadRecorder()
  // The live view sends half the tab's pixels; the tab's capture is not
  // taken down to them, so a recording that follows keeps the tab's own.
  extension.send({ type: 'start', capture: 1, target: 'tab', width: 50, height: 40, sourceWidth: 100, sourceHeight: 80, fps: 15, bitrate: 1000000 })
  await setImmediate()
  extension.send({ type: 'record', capture: 2, target: 'tab', width: 100, height: 80, keyInterval: 120 })
  await setImmediate()
  expect(extension.opened).toEqual([{ maxFrameRate: 120, width: 100, height: 80 }])
})

// Planted defect this catches: frames the browser discarded before the
// extension read them, which no count shows.
test('the frames the browser discarded are reported as missed', async () => {
  const extension = await loadRecorder()
  extension.send({ type: 'record', capture: 1, target: 'tab', width: 100, height: 80, keyInterval: 120 })
  await setImmediate()
  extension.draw(1000, 7)
  await setImmediate()
  extension.stats.discardedFrames = 3
  extension.send({ type: 'stop', capture: 1 })
  await setImmediate()
  await setImmediate()
  expect(extension.sent).toContainEqual({ missed: 3 })
})

// Planted defect this catches: a frame the tab drew again unchanged that is
// coded, or a change of one pixel that is not told with its place.
test('a frame equal to the one before is not coded and a one-pixel change says where it is', async () => {
  const extension = await loadRecorder()
  extension.send({ type: 'record', capture: 1, target: 'tab', width: 100, height: 80, keyInterval: 120 })
  await setImmediate()
  extension.draw(1000, 7)
  extension.draw(17000, 7)
  extension.draw(33000, 7, [5, 3])
  extension.draw(50000, 7, [5, 3])
  await setImmediate()
  // The first frame and the one with a pixel changed; the others are the
  // same as the frame kept before them.
  expect(extension.encoder()?.pending).toEqual([1000, 33000])
  extension.send({ type: 'stop', capture: 1 })
  await setImmediate()
  await setImmediate()
  expect(extension.pictures.length).toBe(2)
  const header = new DataView(extension.pictures[1])
  expect(header.getUint8(8) & 2).toBe(2)
  expect([24, 26, 28, 30].map(at => header.getUint16(at))).toEqual([5, 3, 1, 1])
})

// Planted defect this catches: a key frame only at the start, which leaves the
// end of a long recording to be decoded from its beginning.
test('a key frame comes every interval the Host names and at the first picture', async () => {
  const extension = await loadRecorder()
  extension.send({ type: 'record', capture: 1, target: 'tab', width: 100, height: 80, keyInterval: 3 })
  await setImmediate()
  for (let step = 1; step <= 7; step++) extension.draw(step * 16000, step)
  await setImmediate()
  expect(extension.encoder()?.keys).toEqual([true, false, false, true, false, false, true])
})

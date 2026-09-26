import { expect, test } from 'bun:test'
import { readFile } from 'node:fs/promises'
import { setImmediate } from 'node:timers/promises'
import { runInNewContext } from 'node:vm'

// Runs the capture extension's offscreen document, as `launch.rs` ships it,
// with controlled browser APIs instead of Chrome. `bun run test` runs it.
for (const scenario of ['stopped', 'silent', 'aborted', 'disconnected']) {
  test(`the ${scenario} capture leaves replacement to the Host`, async () => {
    const source = await readFile(new URL('./offscreen.js', import.meta.url), 'utf8')
    const listeners = new Map<string, (event: { data: string }) => void>()
    const timers = new Map<number, { callback: () => void; delay: number }>()
    const grants: string[] = []
    const stopped: string[] = []
    const resets: string[] = []
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
      send(): void {}
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
    runInNewContext(source.replace("import { socket as address } from './config.js';", "const address = 'ws://capture.test';"), {
      WebSocket: Socket,
      MediaStreamTrackProcessor: Processor,
      chrome: { runtime: { sendMessage: async (message: { type: string; target: string }) => {
        if (message.type === 'reset') resets.push(message.type)
        if (message.type !== 'grant') return {}
        grants.push(message.target)
        if (message.target === 'second') await secondGrant
        return { tabId: message.target, streamId: message.target }
      } } },
      navigator: { mediaDevices: { getUserMedia: async (options: { video: { mandatory: { chromeMediaSourceId: string } } }) => {
        if (scenario === 'aborted') throw Object.assign(new Error('Failed due to shutdown'), { name: 'AbortError' })
        const id = options.video.mandatory.chromeMediaSourceId
        const track = { id, stop: () => { stopped.push(id) } }
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
    try {
      message({ data: JSON.stringify({ type: 'start', capture: 1, target: 'first', width: 800, height: 600, fps: 60, bitrate: 1000000 }) })
      await setImmediate()
      if (scenario === 'aborted') {
        expect(resets).toEqual(['reset'])
        expect(grants).toEqual(['first'])
        expect(stopped).toEqual([])
        return
      }
      message({ data: JSON.stringify({ type: 'start', capture: 2, target: 'second', width: 800, height: 600, fps: 60, bitrate: 1000000 }) })
      await setImmediate()
      if (scenario === 'disconnected') {
        if (!socket) throw new Error('extension did not open its socket')
        socket.close()
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
      releaseSecond()
      close({ data: '' })
      for (const end of readers.values()) end()
      await setImmediate()
    }
  })
}

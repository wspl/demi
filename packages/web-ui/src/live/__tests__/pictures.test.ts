import { afterEach, expect, test } from 'bun:test'
import { CanvasPictures } from '../pictures'

/**
 * WebCodecs as a web browser has it, for what the page relies on: a chunk
 * keeps its timestamp as a whole number of microseconds (WebIDL's
 * `long long`), and the decoder's picture carries the chunk's. Bun has no
 * WebCodecs.
 */
const globals = ['VideoDecoder', 'EncodedVideoChunk', 'requestAnimationFrame'] as const

function stubWebCodecs(): void {
  class Chunk {
    readonly type: EncodedVideoChunkType
    readonly timestamp: number
    constructor(init: EncodedVideoChunkInit) {
      this.type = init.type
      this.timestamp = Math.trunc(init.timestamp)
    }
  }
  class Decoder {
    decodeQueueSize = 0
    state: CodecState = 'unconfigured'
    constructor(private readonly init: VideoDecoderInit) {}
    configure(): void {
      this.state = 'configured'
    }
    decode(chunk: Chunk): void {
      this.init.output({ timestamp: chunk.timestamp, displayWidth: 1206, displayHeight: 2622, close() {} } as unknown as VideoFrame)
    }
    close(): void {
      this.state = 'closed'
    }
  }
  const values = { VideoDecoder: Decoder, EncodedVideoChunk: Chunk, requestAnimationFrame: (paint: () => void) => (paint(), 1) }
  for (const name of globals) {
    Object.defineProperty(globalThis, name, { configurable: true, value: values[name] })
  }
}

afterEach(() => {
  for (const name of globals) {
    Reflect.deleteProperty(globalThis, name)
  }
})

/**
 * A picture whose timestamp is a fraction of a microsecond, as a
 * simulator's capture clock gives, is painted. Planted defect this catches: the page finding the
 * decoded picture by the timestamp it sent rather than the one the chunk
 * kept, which drops every such picture. Microseconds.
 */
test('a picture with a fractional timestamp is painted', () => {
  stubWebCodecs()
  const canvas = { width: 0, height: 0, getContext: () => ({ drawImage() {} }) } as unknown as HTMLCanvasElement
  const shown: number[] = []
  const pictures = new CanvasPictures<{ generation: number }>(canvas, {
    shown: (stream) => { shown.push(stream.generation) },
    lost: () => { throw new Error('the stream was lost') },
  })
  pictures.start({ generation: 1 })
  pictures.show({ source: 'm1', generation: 1, sequence: 7, key: true, timestamp: 16236433831.25, width: 1206, height: 2622, data: new Uint8Array([0, 0, 0, 1, 0x65]) })
  expect(shown).toEqual([1])
  expect([canvas.width, canvas.height]).toEqual([1206, 2622])
})

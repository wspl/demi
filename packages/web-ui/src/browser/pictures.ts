/**
 * The pictures of a watched tab on a canvas (`browser-live-view.md` §
 * Delivery): WebCodecs decodes the Host's H.264, the page shows the newest
 * frame it has, and tells the module what it showed.
 */
import type { LiveVideoFrame } from './frames'
import type { PictureSink } from './session'

/** H.264 High 4:2:0, as the Host's extension encodes it. */
const CODEC = 'avc1.640033'
/** Beyond this the page is behind: it drops the stream and asks for a key frame. */
const DECODE_QUEUE = 12

export interface PictureHandlers {
  /** The page painted this frame. */
  shown(generation: number, sequence: number, decodeQueue: number): void
  /** The stream cannot continue; the module must send a key frame. */
  lost(): void
}

/** Whether this browser can show a live view at all. */
export function picturesSupported(): boolean {
  return typeof VideoDecoder === 'function'
}

export class CanvasPictures implements PictureSink {
  private decoder: VideoDecoder | null = null
  private generation = 0
  private latest: { frame: VideoFrame; sequence: number; generation: number } | null = null
  private readonly pending = new Map<number, { sequence: number; generation: number }>()
  private painting: number | null = null

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly handlers: PictureHandlers,
  ) {}

  start(generation: number, width: number, height: number): void {
    this.generation = generation
    this.release()
    this.canvas.width = width
    this.canvas.height = height
  }

  show(frame: LiveVideoFrame): void {
    if (frame.generation !== this.generation) {
      return
    }
    if (!this.decoder) {
      if (!frame.key) {
        this.handlers.lost()
        return
      }
      this.decoder = new VideoDecoder({
        output: (picture) => this.decoded(picture),
        error: () => {
          this.release()
          this.handlers.lost()
        },
      })
      this.decoder.configure({ codec: CODEC, optimizeForLatency: true })
    }
    if (this.decoder.decodeQueueSize > DECODE_QUEUE) {
      this.release()
      this.handlers.lost()
      return
    }
    this.pending.set(frame.timestamp, { sequence: frame.sequence, generation: frame.generation })
    this.decoder.decode(new EncodedVideoChunk({
      type: frame.key ? 'key' : 'delta',
      timestamp: frame.timestamp,
      data: frame.data,
    }))
  }

  stop(): void {
    this.release()
    const context = this.canvas.getContext('2d')
    context?.clearRect(0, 0, this.canvas.width, this.canvas.height)
  }

  private decoded(picture: VideoFrame): void {
    const about = this.pending.get(picture.timestamp)
    this.pending.delete(picture.timestamp)
    if (!about || about.generation !== this.generation) {
      picture.close()
      return
    }
    // Only the newest picture is worth painting.
    this.latest?.frame.close()
    this.latest = { frame: picture, sequence: about.sequence, generation: about.generation }
    this.painting ??= requestAnimationFrame(() => this.paint())
  }

  private paint(): void {
    this.painting = null
    const latest = this.latest
    this.latest = null
    if (!latest) {
      return
    }
    try {
      if (latest.generation !== this.generation) {
        return
      }
      const context = this.canvas.getContext('2d', { alpha: false })
      if (this.canvas.width !== latest.frame.displayWidth || this.canvas.height !== latest.frame.displayHeight) {
        this.canvas.width = latest.frame.displayWidth
        this.canvas.height = latest.frame.displayHeight
      }
      context?.drawImage(latest.frame, 0, 0)
      this.handlers.shown(latest.generation, latest.sequence, this.decoder?.decodeQueueSize ?? 0)
    } finally {
      latest.frame.close()
    }
  }

  private release(): void {
    if (this.decoder && this.decoder.state !== 'closed') {
      this.decoder.close()
    }
    this.decoder = null
    this.latest?.frame.close()
    this.latest = null
    this.pending.clear()
    if (this.painting !== null) {
      cancelAnimationFrame(this.painting)
      this.painting = null
    }
  }
}

/**
 * The pictures of a watched tab on a canvas (`live-view.md` §
 * Delivery): WebCodecs decodes the Host's H.264, the page shows the newest
 * frame it has, and tells the module what it showed.
 */
import { LIVE_VIDEO_CODEC } from '../generated/plugin'
import type { LiveVideoFrame } from './frames'
import type { LiveStream, PictureSink } from './session'

/** The live protocol's codec, as the Host's extension encodes it, decoded with the least delay. */
const DECODER: VideoDecoderConfig = { codec: LIVE_VIDEO_CODEC, optimizeForLatency: true }
/** Beyond this the page is behind: it drops the stream and asks for a key frame. */
const DECODE_QUEUE = 12

export interface PictureHandlers {
  /** The page painted this frame of `stream`'s generation. */
  shown(stream: LiveStream, sequence: number, decodeQueue: number): void
  /** The stream cannot continue; the module must send a key frame. */
  lost(): void
}

/**
 * Whether this web browser can show a live view: its WebCodecs must decode
 * the Host's H.264 (`live-view.md` § A browser tab in the panel). A Chromium
 * built without proprietary codecs has a `VideoDecoder`, but not for H.264.
 */
export async function picturesSupported(defect: (message: string, error: unknown) => void): Promise<boolean> {
  if (typeof VideoDecoder !== 'function') {
    return false
  }
  try {
    const { supported } = await VideoDecoder.isConfigSupported(DECODER)
    return supported === true
  } catch (error) {
    // A web browser refuses to consider only a config it takes for malformed: this page's defect, and no view either way.
    defect('The live view asked about a decoder config the browser refuses', error)
    return false
  }
}

export class CanvasPictures implements PictureSink {
  private decoder: VideoDecoder | null = null
  /** The generation whose frames follow; none before the first. */
  private stream: LiveStream | null = null
  private latest: { frame: VideoFrame; sequence: number; generation: number } | null = null
  private readonly pending = new Map<number, { sequence: number; generation: number }>()
  private painting: number | null = null

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly handlers: PictureHandlers,
  ) {}

  /**
   * Frames of `generation` follow. The canvas keeps what it shows until the
   * first of them is painted, which also gives it their size: sizing or
   * clearing it here would show a black frame between two pictures.
   */
  start(stream: LiveStream): void {
    this.stream = stream
    this.release()
  }

  show(frame: LiveVideoFrame): void {
    if (frame.generation !== this.stream?.generation) {
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
      this.decoder.configure(DECODER)
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

  /** Nothing is watched: decoding ends. The last picture stays until its canvas goes or the next one is painted. */
  stop(): void {
    this.release()
  }

  private decoded(picture: VideoFrame): void {
    const about = this.pending.get(picture.timestamp)
    this.pending.delete(picture.timestamp)
    if (!about || about.generation !== this.stream?.generation) {
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
      const stream = this.stream
      if (!stream || latest.generation !== stream.generation) {
        return
      }
      const context = this.canvas.getContext('2d', { alpha: false })
      if (this.canvas.width !== latest.frame.displayWidth || this.canvas.height !== latest.frame.displayHeight) {
        this.canvas.width = latest.frame.displayWidth
        this.canvas.height = latest.frame.displayHeight
      }
      context?.drawImage(latest.frame, 0, 0)
      this.handlers.shown(stream, latest.sequence, this.decoder?.decodeQueueSize ?? 0)
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

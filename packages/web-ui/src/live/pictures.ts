/**
 * A live view's pictures on a canvas (`mobile.md` § The live view): WebCodecs decodes the Host's H.264 frames in
 * sequence, and the page draws the newest picture it has at the next
 * refresh.
 * Whatever the source, a browser tab or a simulator, its pictures come so.
 */
import { LIVE_VIDEO_CODEC } from '@demicodes/protocol'
import type { LiveVideoFrame } from './frames'

/** The live protocol's codec, as every source encodes it, decoded with the least delay. */
const DECODER: VideoDecoderConfig = { codec: LIVE_VIDEO_CODEC, optimizeForLatency: true }

/** A generation of pictures, as the Host announced it. */
export interface LivePictureStream {
  generation: number
}

/** What shows the pictures: a decoder in the page, or the gallery's canvas. */
export interface PictureSink<S extends LivePictureStream = LivePictureStream> {
  /** Frames of this generation follow. */
  start(stream: S): void
  show(frame: LiveVideoFrame): void
  /** Nothing is watched any more. */
  stop(): void
}

export interface PictureHandlers<S extends LivePictureStream> {
  /** The page painted a picture of `stream`'s generation. */
  shown(stream: S): void
  /** A frame could not be decoded; the Host must send a key frame. */
  lost(): void
}

/**
 * Whether this web browser can show a live view: its WebCodecs must decode
 * the Host's H.264. A Chromium
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

export class CanvasPictures<S extends LivePictureStream> implements PictureSink<S> {
  private decoder: VideoDecoder | null = null
  /** The generation whose frames follow; none before the first. */
  private stream: S | null = null
  private latest: { frame: VideoFrame; generation: number } | null = null
  /** The generation of each frame in the decoder, by the timestamp its picture comes back with. */
  private readonly pending = new Map<number, number>()
  private painting: number | null = null

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly handlers: PictureHandlers<S>,
  ) {}

  /**
   * Frames of `generation` follow. The canvas keeps what it shows until the
   * first of them is painted, which also gives it their size: sizing or
   * clearing it here would show a black frame between two pictures.
   */
  start(stream: S): void {
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
    const chunk = new EncodedVideoChunk({
      type: frame.key ? 'key' : 'delta',
      timestamp: frame.timestamp,
      data: frame.data,
    })
    // The picture comes back with the chunk's timestamp, whole microseconds,
    // which a source's own (a fraction, from a capture clock) is not.
    this.pending.set(chunk.timestamp, frame.generation)
    this.decoder.decode(chunk)
  }

  /** Nothing is watched: decoding ends. The last picture stays until its canvas goes or the next one is painted. */
  stop(): void {
    this.release()
  }

  private decoded(picture: VideoFrame): void {
    const generation = this.pending.get(picture.timestamp)
    this.pending.delete(picture.timestamp)
    if (generation === undefined || generation !== this.stream?.generation) {
      picture.close()
      return
    }
    // Only the newest picture is worth painting.
    this.latest?.frame.close()
    this.latest = { frame: picture, generation }
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
      this.handlers.shown(stream)
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

/**
 * A live view's stream on the page, whatever its source (`live-view.md`
 * § The stream, `mobile.md` § The live view): it opens the plugin's user
 * stream, reads its frames, watches its silence and its stalls, hands the
 * pictures of the generation the view shows to a decoder, acknowledges
 * what the viewer saw and asks for key frames, and opens a view that ended
 * again after the page's reconnect waits (`web-application.md` § Liveness
 * and reconnection). What a source's view says over it is its own: the
 * session that uses a link reads its messages and sends its own.
 */
import type { OpenUserStream, StreamBytes, UserStream } from '../plugins/streams'
import { waitToReconnect, watchSilence, type ReconnectWait, type SilenceWatch } from '../transport/liveness'
import type { LiveVideoFrame } from './frames'
import type { LivePictureStream, PictureSink } from './pictures'
import { LIVE_STALL_MS } from './protocol'

export type LiveConnection = 'opening' | 'live' | 'stalled' | 'ended'

/** Why the page ended a view whose module sent a frame the protocol refuses. */
export const REFUSED_FRAME = 'invalid_frame'
/** Why the page ended a view whose stream brought nothing, heartbeats included, for as long as a page socket may. */
export const SILENT_STREAM = 'silent'

/** What the view shows of its connection; the session's own state holds it. */
export interface LiveLinkState {
  connection: LiveConnection
  /** Why the view ended, until one works again. */
  ended: string | null
}

/** The frames of one stream, as a source's protocol reads them. */
export interface LiveFrames<F> {
  /** The frames `chunk` completes; a frame the protocol refuses throws. */
  read(chunk: Uint8Array): F[]
}

export interface LiveLinkHandlers<F, S extends LivePictureStream> {
  open: OpenUserStream
  /** A reader for a new stream, which reads it from its first byte. */
  frames(): LiveFrames<F>
  /** A stream opened: what the page says first, and what it watched before. */
  opened(): void
  /** A frame the module sent. */
  received(frame: F): void
  /** The view ended for `reason`: what it showed of the module goes. */
  ended(reason: string): void
  /** Nothing arrived for a while: what the viewer holds is let go rather than delivered all at once later. */
  stalled(): void
  /** The generation's pictures are for the view now, as against a source it no longer watches. */
  watching(stream: S): boolean
  /** The page painted `sequence` of `stream`'s generation; the module paces itself by these. */
  acknowledge(stream: S, sequence: number, decodeQueue: number): void
  /** The decoder needs a key frame of `stream`'s generation. */
  keyframe(stream: S): void
  /** Reports a defect of the page, such as a frame the protocol refuses. */
  defect(message: string, error: unknown): void
  now?(): number
}

export class LiveLink<F, S extends LivePictureStream> {
  private stream: UserStream | null = null
  private silence: SilenceWatch | null = null
  private pictures: PictureSink<S> | null = null
  /** The generation the pictures come in now; none before the first. */
  private video: S | null = null
  private received: number
  /** When the page last asked for a key frame; never, at first. */
  private resynced = Number.NEGATIVE_INFINITY
  /** Views that ended in a row since one last worked. */
  private failures = 0
  private reopening: ReconnectWait | null = null

  constructor(
    private readonly state: LiveLinkState,
    private readonly handlers: LiveLinkHandlers<F, S>,
  ) {
    this.received = this.time()
  }

  private time(): number {
    return this.handlers.now ? this.handlers.now() : performance.now()
  }

  /** Whether a stream is open or about to open again, so a new one is not needed. */
  get active(): boolean {
    return this.stream !== null || this.reopening !== null
  }

  /** Whether a stream is open now, to send on. */
  get connected(): boolean {
    return this.stream !== null
  }

  /** The generation the pictures come in now. */
  get picture(): S | null {
    return this.video
  }

  start(): void {
    this.received = this.time()
    const frames = this.handlers.frames()
    // A view ends once: the module's end and the socket's close both say so, and a
    // stream this link already left says nothing about the one that followed it.
    const stream = this.handlers.open({
      data: (bytes) => {
        if (this.stream === stream) {
          this.receive(frames, bytes)
        }
      },
      closed: (reason) => {
        if (this.stream === stream) {
          this.end(reason)
        }
      },
    })
    this.stream = stream
    // The module speaks at least every quarter second, so the page's rule
    // for a silent socket holds for the view as for its other sockets.
    this.silence = watchSilence('live view', () => this.end(SILENT_STREAM))
    this.handlers.opened()
  }

  /** A view waiting to reconnect connects at once, its waits started over. */
  reconnect(): void {
    if (this.reopening === null) {
      return
    }
    this.reopening.cancel()
    this.reopening = null
    this.failures = 0
    this.start()
  }

  /** The page is done with the view: nothing reopens it. */
  close(): void {
    this.reopening?.cancel()
    this.reopening = null
    this.drop()
    this.state.connection = 'ended'
  }

  /** The view ended, as the module or the socket said: it opens again after the page's waits. */
  end(reason: string): void {
    this.drop()
    this.state.ended = reason
    this.handlers.ended(reason)
    this.failures += 1
    this.state.connection = 'opening'
    this.reopening = waitToReconnect(this.failures, () => {
      this.reopening = null
      this.start()
    })
  }

  /** The module answered: the view works. */
  live(): void {
    this.failures = 0
    this.state.ended = null
    if (this.state.connection === 'opening') {
      this.state.connection = 'live'
    }
  }

  send(bytes: StreamBytes): void {
    this.stream?.send(bytes)
  }

  /** Lets the stream go, with its watch and its pictures: nothing it says afterwards reaches the view. */
  private drop(): void {
    this.silence?.stop()
    this.silence = null
    this.pictures?.stop()
    this.video = null
    const stream = this.stream
    this.stream = null
    stream?.close()
  }

  private receive(frames: LiveFrames<F>, bytes: Uint8Array): void {
    this.received = this.time()
    this.silence?.heard()
    if (this.state.connection === 'stalled') {
      this.state.connection = 'live'
      // What the decoder missed while nothing arrived starts again.
      this.resync()
    }
    let read: F[]
    try {
      read = frames.read(bytes)
    } catch (error) {
      // The module ships with this page, so a frame the protocol refuses is its defect.
      this.handlers.defect('The live view received a frame the protocol refuses', error)
      this.end(REFUSED_FRAME)
      return
    }
    for (const frame of read) {
      if (this.stream === null) {
        // A frame ended the view; what came with it is the ended stream's.
        return
      }
      this.handlers.received(frame)
    }
  }

  /** What shows the pictures, once the view has a canvas; a canvas that comes late asks for a key frame. */
  attach(pictures: PictureSink<S>): void {
    this.pictures?.stop()
    this.pictures = pictures
    if (this.video && this.handlers.watching(this.video)) {
      pictures.start(this.video)
      this.resync()
    }
  }

  detach(pictures: PictureSink<S>): void {
    if (this.pictures === pictures) {
      pictures.stop()
      this.pictures = null
    }
  }

  /** Pictures of `stream`'s generation follow. */
  generation(stream: S): void {
    this.video = stream
    this.pictures?.start(stream)
  }

  /** The view watches nothing it had pictures of: decoding ends. */
  unwatched(): void {
    this.pictures?.stop()
    this.video = null
  }

  /** A picture: shown when it is of the generation the view shows; answers whether it was. */
  show(frame: LiveVideoFrame): boolean {
    if (frame.generation !== this.video?.generation) {
      return false
    }
    this.pictures?.show(frame)
    return true
  }

  /** The page painted a picture. */
  showed(generation: number, sequence: number, decodeQueue: number): void {
    if (this.video && generation === this.video.generation) {
      this.handlers.acknowledge(this.video, sequence, decodeQueue)
    }
  }

  /** The decoder lost the stream: the next frame must be a key frame, asked for at most once a second. */
  resync(): void {
    const now = this.time()
    if (!this.video || now - this.resynced < 1000) {
      return
    }
    this.resynced = now
    this.handlers.keyframe(this.video)
  }

  /** Time passes: without a byte for a second the connection is stalled. */
  tick(): void {
    if (this.state.connection === 'live' && this.time() - this.received > LIVE_STALL_MS) {
      this.state.connection = 'stalled'
      this.handlers.stalled()
    }
  }
}

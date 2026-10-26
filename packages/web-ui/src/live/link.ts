/**
 * A live view's stream on the page, whatever its source
 * (`crates/plugin-mobile/page/src/index.ts`).
 *
 * The link:
 *
 * - opens the plugin's user stream and reads its frames,
 * - watches its silence and its stalls,
 * - joins the pictures' chunks and acknowledges each with when it arrived,
 * - hands the frames of the generation the view shows to a decoder in sequence,
 * - asks for a key frame after one it could not decode,
 * - tells the stream whether anyone watches the view, and expects nothing of
 *   a view nobody watches, whose module may fall silent, so it is neither
 *   stalled nor broken for its silence,
 * - ends a view whose module does not answer in time when the session
 *   expects it to, as a session does when its view is watched again: a
 *   connection lost while its module was silent shows only then, and
 * - opens a view that ended again after the page's reconnect waits, or, for
 *   one nobody watches, once someone does: an unwatched view keeps no
 *   conversation, Host or Cloud awake, so the backend may end its stream.
 *
 * What a source's view says over it is its own: the session that uses a link
 * reads its messages and sends its own.
 */
import type { OpenUserStream, StreamBytes, UserStream } from '../plugins/streams'
import { waitToReconnect, watchSilence, type ReconnectWait, type SilenceWatch } from '../transport/liveness'
import { LiveFrameAssembler, encodeAcks, type LiveChunkAck, type LiveVideoChunk } from './frames'
import type { LivePictureStream, PictureSink } from './pictures'
import { LIVE_STALL_MS } from './protocol'
import { pageLog } from '../infra/log'

/** The most acknowledgements one message carries, because a message on the link is limited in size. */
const ACKS_PER_MESSAGE = 8
/** The most bytes the page holds of frames not yet whole and of generations not yet announced, because a link that never completes them would grow the memory without end. */
export const LIVE_HELD_BYTES = 8 * 1024 * 1024
/** The longest a whole frame waits for an earlier one. */
export const LIVE_WAIT_MS = 2000

export type LiveConnection = 'opening' | 'live' | 'stalled' | 'ended'

/** Why the page ended a view whose module sent a frame the protocol refuses. */
export const REFUSED_FRAME = 'invalid_frame'
/** Why the page ended a view whose stream brought nothing, heartbeats included, for as long as the page connection may. */
export const SILENT_STREAM = 'silent'
/** Why the page ended a view whose module did not answer in time when the session expected it to. */
export const UNANSWERED_STREAM = 'unanswered'
/** How long a module may take to answer the session, as the page connection's pong may. */
export const LIVE_ANSWER_MS = 5_000

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
  /** The decoder needs a key frame of `stream`'s generation. */
  keyframe(stream: S): void
  /** Reports a defect of the page, such as a frame the protocol refuses. */
  defect(message: string, error: unknown): void
  /** What the view shows, such as `browser`, which its lines name. */
  readonly view: string
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
  /** Joins the stream's chunks into frames, from its first byte. */
  private frames = new LiveFrameAssembler()
  /**
   * Chunks of generations not announced yet, which may come before their
   * announcement on the direct path, by generation, with their bytes.
   */
  private early = new Map<number, LiveVideoChunk[]>()
  private earlyBytes = 0
  /** The newest generation announced; none before the first. */
  private announced = 0
  /** Acknowledgements of the chunks one task received, which leave together at its end. */
  private acks: LiveChunkAck[] = []
  private acking = false
  /** Views that ended in a row since one last worked. */
  private failures = 0
  private reopening: ReconnectWait | null = null
  /** Whether anyone watches the view; a stream starts watched. */
  private watchedNow = true
  /** The view ended while nobody watched it, and opens again once someone does. */
  private endedUnwatched = false
  /** When the module's answer the session expects is due; none while it expects none. */
  private answerDue: number | null = null
  /**
   * The steps of the stream's way to its first picture, by the page's clock,
   * which one line reports once the picture is drawn.
   */
  private first: { began: number; answered?: number; arrived?: number; frame?: number; written: boolean } | null = null

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
    this.endedUnwatched = false
    this.received = this.time()
    this.first = { began: performance.now(), written: false }
    const frames = this.handlers.frames()
    // A view ends once: the module's end and the stream's close both say so, and a
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
    }, { live: true })
    this.stream = stream
    if (this.watchedNow) {
      this.listen()
    } else {
      stream.watched(false)
    }
    this.handlers.opened()
  }

  /**
   * Expects the module to speak, from now: it speaks at least every quarter
   * second while someone watches the view, so the page's rule for a silent
   * connection holds for the view as for the page connection.
   */
  private listen(): void {
    this.received = this.time()
    this.silence?.stop()
    this.silence = watchSilence('live view', () => this.end(SILENT_STREAM))
  }

  /** A view waiting to reconnect connects at once, its waits started over. */
  reconnect(): void {
    if (this.reopening === null) {
      return
    }
    this.reopening.cancel()
    this.reopening = null
    this.failures = 0
    this.reopen()
  }

  /**
   * Whether anyone watches the view now. The stream is told; a view waiting
   * to open again waits for someone to watch it instead, and a view that
   * ended while nobody watched it opens again at once.
   */
  watched(watched: boolean): void {
    if (watched === this.watchedNow) {
      return
    }
    this.watchedNow = watched
    this.stream?.watched(watched)
    if (watched && this.stream !== null) {
      this.listen()
    } else if (!watched) {
      this.silence?.stop()
      this.silence = null
      this.answerDue = null
    }
    if (!watched && this.reopening !== null) {
      this.reopening.cancel()
      this.reopening = null
      this.endedUnwatched = true
    }
    if (watched && this.endedUnwatched) {
      this.failures = 0
      this.start()
    }
  }

  /**
   * Opens the ended view again while someone watches it; otherwise it waits
   * for someone to: opening it would wake what it ended for, such as a
   * stopped Cloud.
   */
  private reopen(): void {
    if (!this.watchedNow) {
      this.endedUnwatched = true
      return
    }
    this.start()
  }

  /** The page is done with the view: nothing reopens it. */
  close(): void {
    this.reopening?.cancel()
    this.reopening = null
    this.endedUnwatched = false
    this.drop()
    this.state.connection = 'ended'
  }

  /** The view ended, as the module or the stream said: it opens again after the page's waits. */
  end(reason: string): void {
    this.drop()
    this.state.ended = reason
    this.handlers.ended(reason)
    this.failures += 1
    this.state.connection = 'opening'
    if (!this.watchedNow) {
      this.endedUnwatched = true
      return
    }
    this.reopening = waitToReconnect(this.failures, () => {
      this.reopening = null
      this.reopen()
    })
  }

  /**
   * The session asked its module for an answer, which it gives with
   * [`live`](#live): a view that does not hear it within
   * [`LIVE_ANSWER_MS`](#LIVE_ANSWER_MS) ends, and opens again. Only an answer
   * from the module counts, not the backend's word that the stream is up.
   */
  expectAnswer(): void {
    if (this.stream !== null) {
      this.answerDue = this.time() + LIVE_ANSWER_MS
    }
  }

  /** The module answered: the view works. */
  live(): void {
    if (this.first && this.first.answered === undefined) {
      this.first.answered = performance.now()
    }
    this.answerDue = null
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
    this.answerDue = null
    this.silence?.stop()
    this.silence = null
    this.pictures?.stop()
    this.video = null
    this.frames = new LiveFrameAssembler()
    this.early.clear()
    this.earlyBytes = 0
    this.announced = 0
    this.acks = []
    const stream = this.stream
    this.stream = null
    stream?.close()
  }

  private receive(frames: LiveFrames<F>, bytes: Uint8Array): void {
    this.received = this.time()
    this.silence?.heard()
    if (this.state.connection === 'stalled') {
      // Both paths deliver every chunk: the decoder missed nothing.
      this.state.connection = 'live'
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

  /**
   * What shows the pictures, once the view has a canvas. A canvas that comes
   * late missed the key frame its decoder needs, so it asks for one, as for
   * a frame it could not decode: a still page sends no frame after it.
   */
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
    this.announced = Math.max(this.announced, stream.generation)
    this.pictures?.start(stream)
    // The chunks that came ahead of the announcement are its pictures; those
    // of generations before it will never be shown.
    const early = this.early.get(stream.generation) ?? []
    for (const generation of [...this.early.keys()]) {
      if (generation <= stream.generation)
        this.early.delete(generation)
    }
    this.earlyBytes = [...this.early.values()].flat().reduce((bytes, chunk) => bytes + chunk.data.length, 0)
    for (const chunk of early)
      this.assemble(chunk)
  }

  /** The view watches nothing it had pictures of: decoding ends. */
  unwatched(): void {
    this.pictures?.stop()
    this.video = null
  }

  /**
   * A picture's chunk, or a probe's padding: acknowledged with when it arrived,
   * whatever its generation, so the Host's window frees and its probe measures.
   *
   * Its frame is shown once it is whole and its turn came, when it is of the
   * generation the view shows. Answers whether it was of that generation.
   */
  show(chunk: LiveVideoChunk): boolean {
    this.acknowledge({ generation: chunk.generation, sequence: chunk.sequence, chunk: chunk.chunk, padding: chunk.padding, received: this.received })
    if (chunk.padding) {
      // The Host's probe of the path: acknowledged, and nothing to show.
      return false
    }
    if (this.first && this.first.arrived === undefined) {
      this.first.arrived = performance.now()
    }
    if (chunk.generation > this.announced) {
      // Ahead of its announcement, which the other channel still carries.
      const early = this.early.get(chunk.generation) ?? []
      early.push(chunk)
      this.early.set(chunk.generation, early)
      this.earlyBytes += chunk.data.length
      this.bound()
      return false
    }
    if (chunk.generation !== this.video?.generation) {
      return false
    }
    this.assemble(chunk)
    return true
  }

  private assemble(chunk: LiveVideoChunk): void {
    for (const frame of this.frames.add(chunk, this.time())) {
      if (this.first && this.first.frame === undefined) {
        this.first.frame = performance.now()
      }
      this.pictures?.show(frame)
    }
    this.bound()
  }

  /**
   * Past the bytes the page holds or the time a frame waits, what it holds
   * goes, and the decoder starts again from a key frame.
   */
  private bound(): void {
    const since = this.frames.waitingSince
    const late = since !== null && this.time() - since > LIVE_WAIT_MS
    if (!late && this.earlyBytes + this.frames.held <= LIVE_HELD_BYTES) {
      return
    }
    this.frames.clear()
    this.early.clear()
    this.earlyBytes = 0
    this.resync()
  }

  /** Sends `ack` with the others the same task received, a few to a message. */
  private acknowledge(ack: LiveChunkAck): void {
    this.acks.push(ack)
    if (this.acks.length >= ACKS_PER_MESSAGE) {
      this.flush()
      return
    }
    if (!this.acking) {
      this.acking = true
      queueMicrotask(() => {
        this.acking = false
        this.flush()
      })
    }
  }

  private flush(): void {
    if (this.acks.length === 0) {
      return
    }
    const acks = this.acks
    this.acks = []
    this.stream?.send(encodeAcks(acks))
  }

  /**
   * The page drew a picture of the stream: the first is logged with the
   * steps before it, in milliseconds from the stream's start (the module's
   * answer, the first picture's first bytes, the first picture whole and in
   * its turn, and its drawing), and when it came after the page's own load.
   */
  drawn(): void {
    const first = this.first
    if (!first || first.written) {
      return
    }
    first.written = true
    const now = performance.now()
    const since = (at: number | undefined) => (at === undefined ? null : Math.round(at - first.began))
    pageLog.info('live_view.first_drawn', {
      view: this.handlers.view,
      answered_ms: since(first.answered),
      arrived_ms: since(first.arrived),
      frame_ms: since(first.frame),
      drawn_ms: since(now),
      load_ms: Math.round(now),
    })
  }

  /** The decoder could not decode a frame: the next must be a key frame, asked for at most once a second. */
  resync(): void {
    const now = this.time()
    if (!this.video || now - this.resynced < 1000) {
      return
    }
    this.resynced = now
    this.handlers.keyframe(this.video)
  }

  /**
   * Time passes: without a byte for a second the connection is stalled, and
   * without the answer the session expects in time, the view ends.
   */
  tick(): void {
    if (this.answerDue !== null && this.time() > this.answerDue) {
      this.end(UNANSWERED_STREAM)
      return
    }
    this.bound()
    if (this.watchedNow && this.state.connection === 'live' && this.time() - this.received > LIVE_STALL_MS) {
      this.state.connection = 'stalled'
      this.handlers.stalled()
    }
  }
}

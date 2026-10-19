/**
 * The live view's frames on the page's side (`live-view.md` § Framing and
 * versions, `mobile.md` § The live view): the stream carries bytes, so each
 * message is a length, a kind and a payload. Control messages are JSON in
 * the source's protocol, which decodes them; pictures are the live view's
 * own, in chunks of at most 16 KiB that the page joins and acknowledges.
 */
import {
  LIVE_ACK_BYTES,
  LIVE_ACK_FRAME,
  LIVE_CHUNK_DATA_BYTES,
  LIVE_CONTROL_FRAME,
  LIVE_MAX_FRAME_BYTES,
  LIVE_VIDEO_FRAME,
  LIVE_VIDEO_HEADER_BYTES,
  LIVE_VIDEO_SOURCE_BYTES,
} from '@demicodes/protocol'
import type { StreamBytes } from '../plugins/streams'

/** A picture of one source, such as a browser tab, in the generation its Host announced. */
export interface LiveVideoFrame {
  source: string
  generation: number
  sequence: number
  key: boolean
  /** Microseconds on the Host's clock, as the decoder wants them. */
  timestamp: number
  width: number
  height: number
  data: Uint8Array
}

/** A chunk of a picture, as the Host sends it: its part of the frame's data, `chunk` of `chunks`. */
export interface LiveVideoChunk extends LiveVideoFrame {
  chunk: number
  chunks: number
}

/** The page's acknowledgement of one chunk, with when it received it, in milliseconds on its own clock. */
export interface LiveChunkAck {
  generation: number
  sequence: number
  chunk: number
  received: number
}

/** A frame as it came: a control message's JSON payload, or a picture's chunk. */
export type LiveRawFrame =
  | { kind: 'control'; payload: Uint8Array }
  | { kind: 'video'; frame: LiveVideoChunk }

const encoder = new TextEncoder()
const decoder = new TextDecoder()

/** A frame of `kind` around `payload`, ready to send. */
export function framed(kind: number, payload: Uint8Array): StreamBytes {
  const bytes = new Uint8Array(5 + payload.length)
  new DataView(bytes.buffer).setUint32(0, 1 + payload.length)
  bytes[4] = kind
  bytes.set(payload, 5)
  return bytes
}

/** A control message, checked by its protocol before, ready to send. */
export function encodeControl(message: unknown): StreamBytes {
  return framed(LIVE_CONTROL_FRAME, encoder.encode(JSON.stringify(message)))
}

/**
 * A picture in chunks as the Host frames it (`live-view.md` § Framing and
 * versions), for a view without a Host, such as the gallery's.
 */
export function encodeVideo(frame: LiveVideoFrame): StreamBytes {
  const chunks = Math.max(1, Math.ceil(frame.data.length / LIVE_CHUNK_DATA_BYTES))
  const parts: StreamBytes[] = []
  for (let chunk = 0; chunk < chunks; chunk++) {
    const data = frame.data.subarray(chunk * LIVE_CHUNK_DATA_BYTES, (chunk + 1) * LIVE_CHUNK_DATA_BYTES)
    const payload = new Uint8Array(LIVE_VIDEO_HEADER_BYTES + data.length)
    const header = new DataView(payload.buffer)
    payload.set(encoder.encode(frame.source).subarray(0, LIVE_VIDEO_SOURCE_BYTES))
    const fields = LIVE_VIDEO_SOURCE_BYTES
    header.setUint32(fields, frame.generation)
    header.setUint32(fields + 4, frame.sequence)
    header.setUint16(fields + 8, chunk)
    header.setUint16(fields + 10, chunks)
    header.setUint8(fields + 12, frame.key ? 1 : 0)
    header.setFloat64(fields + 16, frame.timestamp)
    header.setUint16(fields + 24, frame.width)
    header.setUint16(fields + 26, frame.height)
    payload.set(data, LIVE_VIDEO_HEADER_BYTES)
    parts.push(framed(LIVE_VIDEO_FRAME, payload))
  }
  const bytes = new Uint8Array(parts.reduce((length, part) => length + part.length, 0))
  let at = 0
  for (const part of parts) {
    bytes.set(part, at)
    at += part.length
  }
  return bytes
}

/** Acknowledgements of chunks, in one frame, ready to send. */
export function encodeAcks(acks: readonly LiveChunkAck[]): StreamBytes {
  const payload = new Uint8Array(acks.length * LIVE_ACK_BYTES)
  const view = new DataView(payload.buffer)
  acks.forEach((ack, index) => {
    const at = index * LIVE_ACK_BYTES
    view.setUint32(at, ack.generation)
    view.setUint32(at + 4, ack.sequence)
    view.setUint16(at + 8, ack.chunk)
    view.setFloat64(at + 12, ack.received)
  })
  return framed(LIVE_ACK_FRAME, payload)
}

/** The acknowledgements of an acknowledgement frame's payload, for a view without a Host. */
export function decodeAcks(payload: Uint8Array): LiveChunkAck[] {
  const view = new DataView(payload.buffer, payload.byteOffset, payload.length)
  const acks: LiveChunkAck[] = []
  for (let at = 0; at + LIVE_ACK_BYTES <= payload.length; at += LIVE_ACK_BYTES) {
    acks.push({
      generation: view.getUint32(at),
      sequence: view.getUint32(at + 4),
      chunk: view.getUint16(at + 8),
      received: view.getFloat64(at + 12),
    })
  }
  return acks
}

/**
 * A picture's chunks joined into its frame, and the frames handed on in
 * sequence (`live-view.md` § Delivery): a direct path delivers chunks out of
 * order, and a frame whose chunks are all in waits only for the frames
 * before it, which the decoder needs first. A key frame needs none before
 * it, and those still waiting behind it are dropped.
 */
export class LiveFrameAssembler {
  private generation: number | null = null
  /** The last frame handed on; none before the first. */
  private last: number | null = null
  private readonly parts = new Map<number, { chunk: LiveVideoChunk; pieces: Array<Uint8Array | undefined>; count: number }>()
  /** Whole frames waiting for an earlier one, with when they became whole. */
  private readonly whole = new Map<number, { frame: LiveVideoFrame; since: number }>()

  /** The frames `chunk`, arrived at `now`, lets go on, in sequence. */
  add(chunk: LiveVideoChunk, now = 0): LiveVideoFrame[] {
    if (chunk.generation !== this.generation) {
      this.generation = chunk.generation
      this.clear()
    }
    if (this.last !== null && chunk.sequence <= this.last) {
      return []
    }
    let part = this.parts.get(chunk.sequence)
    if (!part) {
      part = { chunk, pieces: new Array(chunk.chunks), count: 0 }
      this.parts.set(chunk.sequence, part)
    }
    if (part.pieces[chunk.chunk] === undefined) {
      part.pieces[chunk.chunk] = chunk.data
      part.count++
    }
    if (part.count < chunk.chunks) {
      return []
    }
    this.parts.delete(chunk.sequence)
    const data = new Uint8Array(part.pieces.reduce((length, piece) => length + piece!.length, 0))
    let at = 0
    for (const piece of part.pieces) {
      data.set(piece!, at)
      at += piece!.length
    }
    const { chunk: _chunk, chunks: _chunks, ...header } = part.chunk
    this.whole.set(chunk.sequence, { frame: { ...header, data }, since: now })
    return this.release()
  }

  /** The bytes held: of frames not yet whole, and of whole ones that wait. */
  get held(): number {
    let bytes = 0
    for (const part of this.parts.values()) {
      for (const piece of part.pieces)
        bytes += piece?.length ?? 0
    }
    for (const { frame } of this.whole.values())
      bytes += frame.data.length
    return bytes
  }

  /** Since when the longest-waiting whole frame waits for an earlier one; null while none waits. */
  get waitingSince(): number | null {
    let since: number | null = null
    for (const waiting of this.whole.values())
      since = since === null ? waiting.since : Math.min(since, waiting.since)
    return since
  }

  /** Drops what it holds: the next frame it hands on is a key frame. */
  clear(): void {
    this.last = null
    this.parts.clear()
    this.whole.clear()
  }

  private release(): LiveVideoFrame[] {
    const ready: LiveVideoFrame[] = []
    for (;;) {
      const next = this.last === null ? undefined : this.whole.get(this.last + 1)?.frame
      if (next) {
        this.take(next)
        ready.push(next)
        continue
      }
      // The newest key frame that is whole goes on without what is before it.
      const key = [...this.whole.values()].map(({ frame }) => frame).filter((frame) => frame.key).sort((a, b) => b.sequence - a.sequence)[0]
      if (!key) {
        return ready
      }
      this.take(key)
      ready.push(key)
    }
  }

  private take(frame: LiveVideoFrame): void {
    this.last = frame.sequence
    for (const sequence of [...this.whole.keys(), ...this.parts.keys()]) {
      if (sequence <= frame.sequence) {
        this.whole.delete(sequence)
        this.parts.delete(sequence)
      }
    }
  }
}

/**
 * Bytes split at the live view's frames as they arrive, each frame whole
 * with its length, as the direct path needs them to send a frame on the
 * channel its kind goes on (`live-view.md` § Two paths, one protocol). One
 * splitter splits one stream: a frame an ended stream left unfinished is no
 * part of the next.
 */
export class LiveFrameSplitter {
  private pending: Uint8Array = new Uint8Array(0)

  /** The frames `chunk` completes; a frame of an impossible size throws. */
  split(chunk: Uint8Array): Uint8Array[] {
    const joined = new Uint8Array(this.pending.length + chunk.length)
    joined.set(this.pending)
    joined.set(chunk, this.pending.length)
    this.pending = joined
    const frames: Uint8Array[] = []
    let start = 0
    while (this.pending.length - start >= 4) {
      const view = new DataView(this.pending.buffer, this.pending.byteOffset + start)
      const length = view.getUint32(0)
      if (length === 0 || length > LIVE_MAX_FRAME_BYTES) {
        throw new Error('the live stream sent a frame of an impossible size')
      }
      if (this.pending.length - start < 4 + length) {
        break
      }
      frames.push(this.pending.subarray(start, start + 4 + length))
      start += 4 + length
    }
    this.pending = this.pending.subarray(start)
    return frames
  }
}

/** A frame's kind: its fifth byte, after its length. */
export function frameKind(frame: Uint8Array): number | undefined {
  return frame[4]
}

/**
 * What the Host sent, split from the bytes as they arrive. One reader reads
 * one stream: a frame an ended stream left unfinished is no part of the next.
 */
export class LiveFrameReader {
  private readonly splitter = new LiveFrameSplitter()

  /**
   * The frames `chunk` completes. A frame the protocol refuses throws
   * (`live-view.md` § Framing and versions): it is the Host's defect, and
   * the view it came on ends.
   */
  read(chunk: Uint8Array): LiveRawFrame[] {
    return this.splitter.split(chunk).map((frame) => decodeFrame(frame.subarray(4)))
  }
}

function decodeFrame(frame: Uint8Array): LiveRawFrame {
  const payload = frame.subarray(1)
  if (frame[0] === LIVE_CONTROL_FRAME) {
    return { kind: 'control', payload }
  }
  // The Host's side and the page ship together, so no other kind can come from a newer Host.
  if (frame[0] !== LIVE_VIDEO_FRAME) {
    throw new Error(`the live stream sent a frame of kind ${frame[0]}, which the Host does not send`)
  }
  if (payload.length < LIVE_VIDEO_HEADER_BYTES) {
    throw new Error('the live stream sent a video chunk shorter than its header')
  }
  const header = new DataView(payload.buffer, payload.byteOffset, LIVE_VIDEO_HEADER_BYTES)
  const source = decoder.decode(payload.subarray(0, LIVE_VIDEO_SOURCE_BYTES)).replace(/\0+$/, '')
  const fields = LIVE_VIDEO_SOURCE_BYTES
  const chunks = header.getUint16(fields + 10)
  if (chunks === 0 || header.getUint16(fields + 8) >= chunks) {
    throw new Error('the live stream sent a video chunk outside its frame')
  }
  return {
    kind: 'video',
    frame: {
      source,
      generation: header.getUint32(fields),
      sequence: header.getUint32(fields + 4),
      chunk: header.getUint16(fields + 8),
      chunks: header.getUint16(fields + 10),
      key: (header.getUint8(fields + 12) & 1) === 1,
      timestamp: header.getFloat64(fields + 16),
      width: header.getUint16(fields + 24),
      height: header.getUint16(fields + 26),
      data: payload.subarray(LIVE_VIDEO_HEADER_BYTES),
    },
  }
}

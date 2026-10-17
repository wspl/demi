/**
 * The live view's frames on the page's side (`live-view.md` § Framing and
 * versions, `mobile.md` § The live view): the stream carries bytes, so each
 * message is a length, a kind and a payload. Control messages are JSON in
 * the source's protocol, which decodes them; pictures are the live view's
 * own; a source's other kinds, such as the browser's file frames, go back to
 * it as they came.
 */
import {
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

/** A frame as it came: a control message's JSON payload, a picture, or a source's own kind. */
export type LiveRawFrame =
  | { kind: 'control'; payload: Uint8Array }
  | { kind: 'video'; frame: LiveVideoFrame }

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
 * A picture as the Host frames it (`live-view.md` § The stream), for a view
 * without a Host, such as the gallery's.
 */
export function encodeVideo(frame: LiveVideoFrame): StreamBytes {
  const payload = new Uint8Array(LIVE_VIDEO_HEADER_BYTES + frame.data.length)
  const header = new DataView(payload.buffer)
  payload.set(encoder.encode(frame.source).subarray(0, LIVE_VIDEO_SOURCE_BYTES))
  const fields = LIVE_VIDEO_SOURCE_BYTES
  header.setUint32(fields, frame.generation)
  header.setUint32(fields + 4, frame.sequence)
  header.setUint8(fields + 8, frame.key ? 1 : 0)
  header.setFloat64(fields + 12, frame.timestamp)
  header.setUint16(fields + 20, frame.width)
  header.setUint16(fields + 22, frame.height)
  payload.set(frame.data, LIVE_VIDEO_HEADER_BYTES)
  return framed(LIVE_VIDEO_FRAME, payload)
}

/**
 * What the Host sent, split from the bytes as they arrive. One reader reads
 * one stream: a frame an ended stream left unfinished is no part of the next.
 */
export class LiveFrameReader {
  private pending: Uint8Array = new Uint8Array(0)

  /**
   * The frames `chunk` completes. A frame the protocol refuses throws
   * (`live-view.md` § Framing and versions): it is the Host's defect, and
   * the view it came on ends.
   */
  read(chunk: Uint8Array): LiveRawFrame[] {
    const joined = new Uint8Array(this.pending.length + chunk.length)
    joined.set(this.pending)
    joined.set(chunk, this.pending.length)
    this.pending = joined
    const frames: LiveRawFrame[] = []
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
      frames.push(decodeFrame(this.pending.subarray(start + 4, start + 4 + length)))
      start += 4 + length
    }
    this.pending = this.pending.subarray(start)
    return frames
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
    throw new Error('the live stream sent a video frame shorter than its header')
  }
  const header = new DataView(payload.buffer, payload.byteOffset, LIVE_VIDEO_HEADER_BYTES)
  const source = decoder.decode(payload.subarray(0, LIVE_VIDEO_SOURCE_BYTES)).replace(/\0+$/, '')
  const fields = LIVE_VIDEO_SOURCE_BYTES
  return {
    kind: 'video',
    frame: {
      source,
      generation: header.getUint32(fields),
      sequence: header.getUint32(fields + 4),
      key: (header.getUint8(fields + 8) & 1) === 1,
      timestamp: header.getFloat64(fields + 12),
      width: header.getUint16(fields + 20),
      height: header.getUint16(fields + 22),
      data: payload.subarray(LIVE_VIDEO_HEADER_BYTES),
    },
  }
}

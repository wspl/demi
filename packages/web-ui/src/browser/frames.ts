/**
 * The live view's frames on the page's side (`live-view.md` §
 * Framing and versions): the stream carries bytes, so each message is a
 * length, a kind and a payload.
 */
import {
  LIVE_CONTROL_FRAME,
  LIVE_FILE_FRAME,
  LIVE_FILE_HEADER_BYTES,
  LIVE_MAX_FRAME_BYTES,
  LIVE_VIDEO_FRAME,
  LIVE_VIDEO_HEADER_BYTES,
  liveModuleMessageSchema,
  type LiveModuleMessage,
  type LiveViewerMessage,
} from '@demicodes/protocol'
import { z } from 'zod'

/** A picture of one tab, in the generation the module announced. */
export interface LiveVideoFrame {
  tab: string
  generation: number
  sequence: number
  key: boolean
  /** Microseconds on the Host's clock, as the decoder wants them. */
  timestamp: number
  width: number
  height: number
  data: Uint8Array
}

/** Bytes the page sends: their own buffer, as a socket requires. */
export type LiveBytes = Uint8Array<ArrayBuffer>

export type LiveFrame =
  | { kind: 'message'; message: LiveModuleMessage }
  | { kind: 'video'; frame: LiveVideoFrame }

const encoder = new TextEncoder()
const decoder = new TextDecoder()
/** JSON is UTF-8, so a control message that is not is refused rather than repaired. */
const strictDecoder = new TextDecoder('utf-8', { fatal: true })

function framed(kind: number, payload: Uint8Array): LiveBytes {
  const bytes = new Uint8Array(5 + payload.length)
  new DataView(bytes.buffer).setUint32(0, 1 + payload.length)
  bytes[4] = kind
  bytes.set(payload, 5)
  return bytes
}

/** One of the viewer's messages, ready to send. */
export function encodeMessage(message: LiveViewerMessage): LiveBytes {
  return framed(LIVE_CONTROL_FRAME, encoder.encode(JSON.stringify(message)))
}

/** Bytes of the `file`th file of `upload`. */
export function encodeFile(upload: number, file: number, data: Uint8Array): LiveBytes {
  const payload = new Uint8Array(LIVE_FILE_HEADER_BYTES + data.length)
  const header = new DataView(payload.buffer)
  header.setUint32(0, upload)
  header.setUint32(4, file)
  payload.set(data, LIVE_FILE_HEADER_BYTES)
  return framed(LIVE_FILE_FRAME, payload)
}

/**
 * What the module sent, split from the bytes as they arrive. One reader reads
 * one stream: a frame an ended stream left unfinished is no part of the next.
 */
export class LiveFrameReader {
  private pending: Uint8Array = new Uint8Array(0)

  /**
   * The frames `chunk` completes. A frame the protocol refuses throws
   * (`live-view.md` § Framing and versions): it is the module's defect, and
   * the view it came on ends.
   */
  read(chunk: Uint8Array): LiveFrame[] {
    const joined = new Uint8Array(this.pending.length + chunk.length)
    joined.set(this.pending)
    joined.set(chunk, this.pending.length)
    this.pending = joined
    const frames: LiveFrame[] = []
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

function decodeFrame(frame: Uint8Array): LiveFrame {
  const payload = frame.subarray(1)
  if (frame[0] === LIVE_CONTROL_FRAME) {
    return { kind: 'message', message: decodeMessage(payload) }
  }
  // The module and the page ship together, so no other kind can come from a newer module.
  if (frame[0] !== LIVE_VIDEO_FRAME) {
    throw new Error(`the live stream sent a frame of kind ${frame[0]}, which the module does not send`)
  }
  if (payload.length < LIVE_VIDEO_HEADER_BYTES) {
    throw new Error('the live stream sent a video frame shorter than its header')
  }
  const header = new DataView(payload.buffer, payload.byteOffset, LIVE_VIDEO_HEADER_BYTES)
  const tab = decoder.decode(payload.subarray(0, 24)).replace(/\0+$/, '')
  return {
    kind: 'video',
    frame: {
      tab,
      generation: header.getUint32(24),
      sequence: header.getUint32(28),
      key: (header.getUint8(32) & 1) === 1,
      timestamp: header.getFloat64(36),
      width: header.getUint16(44),
      height: header.getUint16(46),
      data: payload.subarray(LIVE_VIDEO_HEADER_BYTES),
    },
  }
}

/** A control message, checked against the protocol before the page acts on it. */
function decodeMessage(payload: Uint8Array): LiveModuleMessage {
  let value: unknown
  try {
    value = JSON.parse(strictDecoder.decode(payload))
  } catch (error) {
    throw new Error('the live stream sent a control message that is not JSON', { cause: error })
  }
  const checked = liveModuleMessageSchema.safeParse(value)
  if (!checked.success) {
    throw new Error(`the live stream sent a control message the protocol refuses:\n${z.prettifyError(checked.error)}`)
  }
  return checked.data
}

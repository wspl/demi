/**
 * The live view's frames on the page's side (`live-view.md` §
 * Framing and versions): the stream carries bytes, so each message is a
 * length, a kind and a payload.
 */
import { liveModuleMessageSchema, type LiveModuleMessage, type LiveViewerMessage } from '../generated/plugin'
import { LIVE_CONTROL_FRAME, LIVE_FILE_FRAME, LIVE_FILE_HEADER_BYTES, LIVE_MAX_FRAME_BYTES, LIVE_VIDEO_FRAME, LIVE_VIDEO_HEADER_BYTES, LIVE_VIDEO_TAB_BYTES } from '../generated/live'
import { z } from 'zod'
import type { StreamBytes } from '@demicodes/plugin-sdk'

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


export type LiveFrame =
  | { kind: 'message'; message: LiveModuleMessage }
  | { kind: 'video'; frame: LiveVideoFrame }

const encoder = new TextEncoder()
const decoder = new TextDecoder()
/** JSON is UTF-8, so a control message that is not is refused rather than repaired. */
const strictDecoder = new TextDecoder('utf-8', { fatal: true })

function framed(kind: number, payload: Uint8Array): StreamBytes {
  const bytes = new Uint8Array(5 + payload.length)
  new DataView(bytes.buffer).setUint32(0, 1 + payload.length)
  bytes[4] = kind
  bytes.set(payload, 5)
  return bytes
}

/** One of the viewer's messages, ready to send. */
export function encodeMessage(message: LiveViewerMessage): StreamBytes {
  return framed(LIVE_CONTROL_FRAME, encoder.encode(JSON.stringify(message)))
}

/**
 * A picture as the module frames it (`live-view.md` § The stream), for a
 * view without a Host, such as the gallery's.
 */
export function encodeVideo(frame: LiveVideoFrame): StreamBytes {
  const payload = new Uint8Array(LIVE_VIDEO_HEADER_BYTES + frame.data.length)
  const header = new DataView(payload.buffer)
  payload.set(encoder.encode(frame.tab).subarray(0, LIVE_VIDEO_TAB_BYTES))
  const fields = LIVE_VIDEO_TAB_BYTES
  header.setUint32(fields, frame.generation)
  header.setUint32(fields + 4, frame.sequence)
  header.setUint8(fields + 8, frame.key ? 1 : 0)
  header.setFloat64(fields + 12, frame.timestamp)
  header.setUint16(fields + 20, frame.width)
  header.setUint16(fields + 22, frame.height)
  payload.set(frame.data, LIVE_VIDEO_HEADER_BYTES)
  return framed(LIVE_VIDEO_FRAME, payload)
}

/** Bytes of the `file`th file of `upload`. */
export function encodeFile(upload: number, file: number, data: Uint8Array): StreamBytes {
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
  const tab = decoder.decode(payload.subarray(0, LIVE_VIDEO_TAB_BYTES)).replace(/\0+$/, '')
  const fields = LIVE_VIDEO_TAB_BYTES
  return {
    kind: 'video',
    frame: {
      tab,
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

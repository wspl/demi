/**
 * The `preview` stream's frames on the page's side (`preview.md` § The
 * stream): framed as the live view's are, a length, a kind and a payload.
 * Control messages are JSON; bodies and socket messages are binary frames
 * behind a small header.
 */
import { z } from 'zod'
import type { StreamBytes } from '@demicodes/plugin-sdk'
import {
  PREVIEW_BODY_CHUNK_BYTES,
  PREVIEW_BODY_HEADER_BYTES,
  PREVIEW_CHUNK_FRAME,
  PREVIEW_CONTROL_FRAME,
  PREVIEW_MAX_FRAME_BYTES,
  PREVIEW_REQUEST_BODY_FRAME,
  PREVIEW_SOCKET_HEADER_BYTES,
  PREVIEW_SOCKET_MESSAGE_FRAME,
  previewEngineMessageSchema,
  type PreviewEngineMessage,
  type PreviewRelayMessage,
} from '../generated/plugin'

export type PreviewFrame =
  | { kind: 'message'; message: PreviewEngineMessage }
  | { kind: 'chunk'; id: number; data: Uint8Array }
  | { kind: 'socket'; id: number; binary: boolean; data: Uint8Array }

const encoder = new TextEncoder()
/** JSON and a text message are UTF-8: what is not is refused, never repaired. */
const strictDecoder = new TextDecoder('utf-8', { fatal: true })

function framed(kind: number, payload: Uint8Array): StreamBytes {
  const bytes = new Uint8Array(5 + payload.length)
  new DataView(bytes.buffer).setUint32(0, 1 + payload.length)
  bytes[4] = kind
  bytes.set(payload, 5)
  return bytes
}

/** One of the relay's messages, ready to send. */
export function encodeMessage(message: PreviewRelayMessage): StreamBytes {
  return framed(PREVIEW_CONTROL_FRAME, encoder.encode(JSON.stringify(message)))
}

/** A chunk of a request's body, at most one frame's worth; an empty one ends it. */
export function encodeRequestBody(id: number, data: Uint8Array): StreamBytes {
  if (data.length > PREVIEW_BODY_CHUNK_BYTES) {
    throw new Error('a request body chunk is larger than a frame carries')
  }
  const payload = new Uint8Array(PREVIEW_BODY_HEADER_BYTES + data.length)
  new DataView(payload.buffer).setUint32(0, id)
  payload.set(data, PREVIEW_BODY_HEADER_BYTES)
  return framed(PREVIEW_REQUEST_BODY_FRAME, payload)
}

/** One message of a page's WebSocket. */
export function encodeSocketMessage(id: number, data: string | Uint8Array): StreamBytes {
  const binary = typeof data !== 'string'
  const bytes = binary ? data : encoder.encode(data)
  const payload = new Uint8Array(PREVIEW_SOCKET_HEADER_BYTES + bytes.length)
  const header = new DataView(payload.buffer)
  header.setUint32(0, id)
  header.setUint8(4, binary ? 1 : 0)
  payload.set(bytes, PREVIEW_SOCKET_HEADER_BYTES)
  return framed(PREVIEW_SOCKET_MESSAGE_FRAME, payload)
}

/**
 * What the engine sent, split from the bytes as they arrive. One reader
 * reads one stream: a frame an ended stream left unfinished is no part of
 * the next.
 */
export class PreviewFrameReader {
  private pending: Uint8Array = new Uint8Array(0)

  /**
   * The frames `chunk` completes. A frame the protocol refuses throws: it is
   * the engine's defect, and the stream it came on ends.
   */
  read(chunk: Uint8Array): PreviewFrame[] {
    const joined = new Uint8Array(this.pending.length + chunk.length)
    joined.set(this.pending)
    joined.set(chunk, this.pending.length)
    this.pending = joined
    const frames: PreviewFrame[] = []
    let start = 0
    while (this.pending.length - start >= 4) {
      const length = new DataView(this.pending.buffer, this.pending.byteOffset + start).getUint32(0)
      if (length === 0 || length > PREVIEW_MAX_FRAME_BYTES) {
        throw new Error('the preview stream sent a frame of an impossible size')
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

function decodeFrame(frame: Uint8Array): PreviewFrame {
  const payload = frame.subarray(1)
  switch (frame[0]) {
    case PREVIEW_CONTROL_FRAME:
      return { kind: 'message', message: decodeMessage(payload) }
    case PREVIEW_CHUNK_FRAME: {
      if (payload.length < PREVIEW_BODY_HEADER_BYTES) {
        throw new Error('the preview stream sent a chunk shorter than its header')
      }
      const data = payload.subarray(PREVIEW_BODY_HEADER_BYTES)
      if (data.length > PREVIEW_BODY_CHUNK_BYTES) {
        throw new Error('the preview stream sent a chunk larger than a frame carries')
      }
      return { kind: 'chunk', id: new DataView(payload.buffer, payload.byteOffset).getUint32(0), data }
    }
    case PREVIEW_SOCKET_MESSAGE_FRAME: {
      if (payload.length < PREVIEW_SOCKET_HEADER_BYTES) {
        throw new Error('the preview stream sent a socket message shorter than its header')
      }
      const header = new DataView(payload.buffer, payload.byteOffset, PREVIEW_SOCKET_HEADER_BYTES)
      const flag = header.getUint8(4)
      if (flag > 1) {
        throw new Error(`the preview stream sent a socket message flagged ${flag}`)
      }
      const data = payload.subarray(PREVIEW_SOCKET_HEADER_BYTES)
      if (flag === 0) {
        // Checked here, so a text message that is not UTF-8 ends the stream as the engine's does.
        strictDecoder.decode(data)
      }
      return { kind: 'socket', id: header.getUint32(0), binary: flag === 1, data }
    }
    default:
      // The engine and the page ship together, so no other kind can come from a newer engine.
      throw new Error(`the preview stream sent a frame of kind ${frame[0]}, which the engine does not send`)
  }
}

/** A control message, checked against the protocol before the relay acts on it. */
function decodeMessage(payload: Uint8Array): PreviewEngineMessage {
  let value: unknown
  try {
    value = JSON.parse(strictDecoder.decode(payload))
  } catch (error) {
    throw new Error('the preview stream sent a control message that is not JSON', { cause: error })
  }
  const checked = previewEngineMessageSchema.safeParse(value)
  if (!checked.success) {
    throw new Error(`the preview stream sent a control message the protocol refuses:\n${z.prettifyError(checked.error)}`)
  }
  return checked.data
}

/** A text socket message's text. */
export function socketText(data: Uint8Array): string {
  return strictDecoder.decode(data)
}

/**
 * The browser's frames on the page's side (`live-view.md` § Framing and
 * versions): the live view's frames, read by the SDK's reader, with control
 * messages in the browser's protocol, and the bytes of files the viewer
 * chose.
 */
import { liveModuleMessageSchema, type LiveModuleMessage, type LiveViewerMessage } from '../generated/plugin'
import { LIVE_FILE_FRAME, LIVE_FILE_HEADER_BYTES } from '../generated/plugin'
import { z } from 'zod'
import { LiveFrameReader as RawFrameReader, encodeControl, framed, type LiveVideoChunk, type StreamBytes } from '@demicodes/plugin-sdk'

export type LiveFrame =
  | { kind: 'message'; message: LiveModuleMessage }
  | { kind: 'video'; frame: LiveVideoChunk }

/** JSON is UTF-8, so a control message that is not is refused rather than repaired. */
const strictDecoder = new TextDecoder('utf-8', { fatal: true })

/** One of the viewer's messages, ready to send. */
export function encodeMessage(message: LiveViewerMessage): StreamBytes {
  return encodeControl(message)
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
  private readonly raw = new RawFrameReader()

  /**
   * The frames `chunk` completes. A frame the protocol refuses throws
   * (`live-view.md` § Framing and versions): it is the module's defect, and
   * the view it came on ends.
   */
  read(chunk: Uint8Array): LiveFrame[] {
    return this.raw.read(chunk).map((frame) =>
      frame.kind === 'video' ? frame : { kind: 'message', message: decodeMessage(frame.payload) })
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

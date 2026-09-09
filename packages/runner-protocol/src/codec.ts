import { decode, encode } from '@msgpack/msgpack'
import type { MessagePackCodec } from './messages'

/**
 * Shared MessagePack implementation for the backend and the txiki.js runner.
 */
export const msgpackCodec: MessagePackCodec = {
  encode: (value) => encode(value),
  decode: (bytes) => decode(bytes),
}

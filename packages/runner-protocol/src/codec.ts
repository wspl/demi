import { decode, encode } from '@msgpack/msgpack'
import type { MessagePackCodec } from './messages'

/**
 * The MessagePack codec of the Bun ends (the backend; the Bun runner until
 * its port). Its defaults are the mapping `tinyjs:bytes` implements, so the
 * two runtimes read each other's frames.
 */
export const msgpackCodec: MessagePackCodec = {
  encode: (value) => encode(value),
  decode: (bytes) => decode(bytes),
}

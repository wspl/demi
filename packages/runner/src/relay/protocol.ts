// The local relay's wire (`runner.md` § The local relay): MessagePack
// frames, each preceded by its length as a 32-bit big-endian integer, on
// the Unix domain socket between a command-mode process and the runner.
import { z } from 'zod'

const bytesSchema = z.custom<Uint8Array>((value) => value instanceof Uint8Array)

/** A command-mode process → the runner. */
export const relayRequestSchema = z.discriminatedUnion('type', [
  /** The manifest, on a cache miss. */
  z.object({ type: z.literal('manifest') }),
  /** An `rpc` invocation with the pipe's bytes; the live stdin follows as `stdin` frames. */
  z.object({
    type: z.literal('rpc'),
    agentSessionId: z.string(),
    shellId: z.string(),
    root: z.string(),
    path: z.array(z.string()),
    argv: z.array(z.string()),
    args: z.record(z.string(), z.unknown()),
    json: z.boolean(),
    cwd: z.string(),
    env: z.record(z.string(), z.string()),
    stdin: bytesSchema,
  }),
  z.object({ type: z.literal('stdin'), bytes: bytesSchema }),
  z.object({ type: z.literal('stdin_end') }),
])

/** The runner → a command-mode process. */
export const relayReplySchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('manifest'), manifest: z.unknown() }),
  z.object({ type: z.literal('output'), stream: z.enum(['stdout', 'stderr']), bytes: bytesSchema }),
  z.object({ type: z.literal('exit'), exitCode: z.number() }),
  z.object({ type: z.literal('error'), message: z.string() }),
])

export type RelayRequest = z.infer<typeof relayRequestSchema>
export type RelayReply = z.infer<typeof relayReplySchema>

export interface RelayCodec {
  encode(value: unknown): Uint8Array
  decode(bytes: Uint8Array): unknown
}

export function frameOf(codec: RelayCodec, value: unknown): Uint8Array {
  const body = codec.encode(value)
  const frame = new Uint8Array(4 + body.byteLength)
  new DataView(frame.buffer).setUint32(0, body.byteLength)
  frame.set(body, 4)
  return frame
}

/** Splits a byte stream into decoded frames. */
export async function* framesOf<T>(input: AsyncIterable<Uint8Array>, codec: RelayCodec, schema: z.ZodType<T>): AsyncIterable<T> {
  let buffer = new Uint8Array(0)
  for await (const chunk of input) {
    const joined = new Uint8Array(buffer.byteLength + chunk.byteLength)
    joined.set(buffer)
    joined.set(chunk, buffer.byteLength)
    buffer = joined
    for (;;) {
      if (buffer.byteLength < 4) break
      const length = new DataView(buffer.buffer, buffer.byteOffset, 4).getUint32(0)
      if (buffer.byteLength < 4 + length) break
      const body = buffer.subarray(4, 4 + length)
      buffer = buffer.subarray(4 + length)
      yield schema.parse(codec.decode(body))
    }
  }
}

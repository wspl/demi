import { z } from 'zod'
import contract from './local-contract.json'

/** Shared by the TS decoder and the generated C header. Length excludes the type byte. */
export const LOCAL = contract
const id = z.string().regex(/^[a-f0-9]{32}$/)
export const localInvokeSchema = z.object({
  version: z.literal(LOCAL.version),
  id,
  context: id,
  root: z.string().min(1),
  argv: z.array(z.string()),
  cwd: z.string().min(1),
  env: z.record(z.string(), z.string()),
  live: z.boolean(),
}).strict()
export const localWatchSchema = z.object({ version: z.literal(LOCAL.version), id, context: id }).strict()
export const localManageSchema = z.object({ version: z.literal(LOCAL.version), secret: id, action: z.enum(['status', 'drain']) }).strict()
export type LocalInvoke = z.infer<typeof localInvokeSchema>

export function localFrame(type: number, body: Uint8Array = new Uint8Array()): Uint8Array {
  if (body.length > LOCAL.maxFrame) throw new Error('local IPC frame exceeds limit')
  const bytes = new Uint8Array(5 + body.length)
  new DataView(bytes.buffer).setUint32(0, body.length)
  bytes[4] = type
  bytes.set(body, 5)
  return bytes
}

export async function* localFrames(input: AsyncIterable<Uint8Array>): AsyncIterable<{ type: number; body: Uint8Array }> {
  let buffer = new Uint8Array(0)
  for await (const chunk of input) {
    const joined = new Uint8Array(buffer.length + chunk.length)
    joined.set(buffer); joined.set(chunk, buffer.length); buffer = joined
    while (buffer.length >= 5) {
      const length = new DataView(buffer.buffer, buffer.byteOffset, 4).getUint32(0)
      if (length > LOCAL.maxFrame) throw new Error('local IPC frame exceeds limit')
      if (buffer.length < length + 5) break
      const type = buffer[4]!
      if (!Object.values(LOCAL.frames).includes(type)) throw new Error('unknown local IPC frame')
      const body = buffer.slice(5, length + 5)
      buffer = buffer.slice(length + 5)
      yield { type, body }
    }
  }
  if (buffer.length) throw new Error('truncated local IPC frame')
}

// What `bun browse` and the slot's tool server say over the server's socket:
// one JSON line with the call, then lines of output and the exit code.
import { z } from 'zod'

export const requestSchema = z.object({
  script: z.string(),
  /** The file the caller named the script by, which a failure names; null for standard input. */
  file: z.string().nullable(),
  headed: z.boolean(),
  /** The caller's `DEMI_*` variables, which the processes the tool starts get. */
  env: z.record(z.string(), z.string()),
  /** How long the call may run. */
  limitMs: z.number().int().positive(),
})
export type Request = z.infer<typeof requestSchema>

export const replySchema = z.union([
  z.object({ out: z.string() }),
  z.object({ err: z.string() }),
  z.object({ exit: z.number().int() }),
  /** The tool's code changed since the server started: it ends, and the caller asks a new one. */
  z.object({ restart: z.literal(true) }),
])
export type Reply = z.infer<typeof replySchema>

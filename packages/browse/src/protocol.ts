// What `bun browse` and the slot's daemon say over the daemon's socket: one
// JSON line with the command line, then lines of output and the exit code.
import { z } from 'zod'

export const requestSchema = z.object({
  argv: z.array(z.string()),
  headed: z.boolean(),
  env: z.record(z.string(), z.string()),
})
export type Request = z.infer<typeof requestSchema>

export const replySchema = z.union([
  z.object({ out: z.string() }),
  z.object({ err: z.string() }),
  z.object({ exit: z.number().int() }),
  /** The tool's code changed since the daemon started: it ends, and the caller asks a new one. */
  z.object({ restart: z.literal(true) }),
])
export type Reply = z.infer<typeof replySchema>

import { z } from 'zod'
import type { ShellEditedFile } from '@demicodes/shell'
import type { ShellToolView } from '../tools'

/**
 * One file a command edited, as the runner reported it and the change store
 * retained it: the same shape in a live block and in cold history.
 */
export const shellEditedFileSchema: z.ZodType<ShellEditedFile> = z.strictObject({
  path: z.string().min(1),
  kind: z.enum(['added', 'modified']),
  added: z.number().int().nonnegative(),
  removed: z.number().int().nonnegative(),
  edits: z.array(z.strictObject({ kept: z.boolean() })).min(1),
})

/**
 * The `view` a shell tool call stores on its block. `shellToolView` in
 * `../tools` produces it; a transcript is storage, so every reader — the
 * server's replay, the page's terminal list, the change pills, a fork's copy
 * of retained edits — validates it against this one declaration, and a view
 * from another tool is simply not one.
 */
export const shellToolViewSchema: z.ZodType<ShellToolView> = z.object({
  kind: z.literal('shell'),
  status: z.enum(['running', 'exited', 'aborted']),
  shellId: z.string(),
  commandId: z.string(),
  exitCode: z.number().optional(),
  runningMs: z.number(),
  idleMs: z.number(),
  chunks: z.array(z.object({
    stream: z.enum(['stdout', 'stderr']),
    text: z.string(),
  })),
  viewTruncated: z.boolean(),
  files: z.array(shellEditedFileSchema).optional(),
  filesTruncated: z.boolean().optional(),
})

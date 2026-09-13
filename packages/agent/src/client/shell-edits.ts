import { z } from 'zod'
import type { ShellEditedFile } from '@demicodes/shell'

/** Retained edit metadata is the same in live blocks and cold history. */
export const shellEditedFileSchema: z.ZodType<ShellEditedFile> = z.object({
  path: z.string().min(1),
  kind: z.enum(['added', 'modified']),
  added: z.number().int().nonnegative(),
  removed: z.number().int().nonnegative(),
  edits: z.array(z.object({ kept: z.boolean() }).strict()).min(1),
}).strict()

export const shellEditsViewSchema = z.object({
  kind: z.literal('shell'),
  commandId: z.string().min(1),
  files: z.array(shellEditedFileSchema),
  filesTruncated: z.boolean().optional(),
})

export type ShellEditsView = z.infer<typeof shellEditsViewSchema>

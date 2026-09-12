import { z } from 'zod'
import {
  editReceiptsSchema,
  modelSelectionSchema,
  queuedMessageSchema,
  sessionPhaseSchema,
} from '../protocol/schemas'

/** Harness state stays opaque here; the harness owns its restore schema. */
export const sessionStateSchema = z.object({
  state: z.unknown().refine((value): boolean => value !== undefined, 'State must be present'),
  phase: sessionPhaseSchema,
  queue: z.array(queuedMessageSchema),
  cwd: z.string(),
  model: modelSelectionSchema,
  harnessName: z.string().min(1),
  edits: editReceiptsSchema.optional(),
})

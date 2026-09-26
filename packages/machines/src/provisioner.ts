import { z } from 'zod'

export const imageStateSchema = z.object({
  generation: z.string().regex(/^[A-Za-z0-9_-]+$/),
  baseVersion: z.string().regex(/^[A-Za-z0-9_-]+$/),
  resetId: z.string().nullable(),
  systemBytes: z.number().int().positive(),
  homeBytes: z.number().int().positive(),
}).strict()
export type MachineImageState = z.infer<typeof imageStateSchema>

import { z } from 'zod'

/** The private, temporary credential file supplied to a managed runner. */
export const managedBootSchema = z.strictObject({
  backendUrl: z.string().regex(/^https?:\/\/[^\s]+$/),
  deviceToken: z.string().min(1).max(4096).regex(/^\S+$/),
})

export type ManagedBoot = z.infer<typeof managedBootSchema>

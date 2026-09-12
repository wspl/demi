import { z } from 'zod'

/** Serializable configuration accepted by the public Grok config parser. */
export const grokBuildConfigSchema = z.strictObject({
  grokHome: z.string().min(1).optional(),
  baseUrl: z.url({ protocol: /^https?$/ }).optional(),
  headers: z.record(z.string(), z.string()).optional(),
})

export type GrokBuildProviderConfig = z.infer<typeof grokBuildConfigSchema>

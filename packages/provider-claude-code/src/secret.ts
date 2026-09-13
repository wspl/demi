/**
 * The `oauth.json` Demi owns for one Claude Code credential: the login flow
 * writes it, the auth store reads it back and renews it in place.
 *
 * Unknown keys are kept, so a secret written by a newer login survives the
 * read/renew/write round trip of an older one.
 */
import { z } from 'zod'

export const claudeCodeOAuthSecretSchema = z.looseObject({
  accessToken: z.string().min(1),
  refreshToken: z.string().min(1).nullish(),
  /** ISO-8601 access token expiry. */
  expiresAt: z.string().min(1).nullish(),
  scopes: z.array(z.string()).nullish(),
  subscriptionType: z.string().min(1).nullish(),
  rateLimitTier: z.string().min(1).nullish(),
  emailAddress: z.string().min(1).nullish(),
})

export type ClaudeCodeOAuthSecret = z.infer<typeof claudeCodeOAuthSecretSchema>

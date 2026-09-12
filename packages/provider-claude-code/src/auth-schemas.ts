import { z } from 'zod'
import { ProviderDataError, parseProviderData, parseProviderJson } from '@demicodes/provider'
import { errorMessage } from '@demicodes/utils'

export const claudeTokenSchema = z.string().min(1).regex(/^\S+$/)
const text = z.string().min(1).regex(/\S/)
const optionalText = text.nullable().optional()
const metadataShape = {
  subscriptionType: optionalText,
  rateLimitTier: optionalText,
}
export const claudeOAuthSecretSchema = z.looseObject({
  accessToken: claudeTokenSchema,
  refreshToken: claudeTokenSchema.nullable().optional(),
  expiresAt: z.iso.datetime({ offset: true }).nullable().optional(),
  scopes: z.array(claudeTokenSchema).nullable().optional(),
  ...metadataShape,
  emailAddress: optionalText,
})
export const claudeOAuthAccessSchema = claudeOAuthSecretSchema.pick({
  accessToken: true, subscriptionType: true, rateLimitTier: true,
}).strip().extend({ source: z.enum(['static', 'file', 'env', 'keychain']) })

// The vendor keychain stores an epoch in milliseconds, unlike Demi's ISO expiry.
const keychainOAuthSchema = claudeOAuthSecretSchema.extend({
  expiresAt: z.number().int().nonnegative().max(8_640_000_000_000_000).nullable().optional(),
})
export const claudeKeychainSchema = z.looseObject({
  claudeAiOauth: keychainOAuthSchema.nullable().optional(),
})
export const claudeTokenResponseSchema = z.looseObject({
  access_token: claudeTokenSchema,
  refresh_token: claudeTokenSchema.optional(),
  expires_in: z.number().int().nonnegative().refine((seconds) =>
    Number.isFinite(new Date(Date.now() + seconds * 1000).getTime()),
  ).optional(),
  scope: z.string().regex(/^(?:[^\s]+(?: +[^\s]+)*)?$/).optional(),
  subscription_type: text.optional(),
  account: z.looseObject({ email_address: text.optional(), subscription_type: text.optional() }).optional(),
})
export const claudeCredentialAddSchema = z.union([
  claudeOAuthSecretSchema.strict(),
  z.strictObject({ oauth: claudeOAuthSecretSchema }),
])
export const claudeAuthorizationCodeSchema = z.string().trim().regex(/^[^#\s]+(?:#[^#\s]+)?$/)

export type ClaudeCodeOAuthSecret = z.infer<typeof claudeOAuthSecretSchema>
export type ClaudeCodeOAuthAccess = z.infer<typeof claudeOAuthAccessSchema>

export class ClaudeCodeAuthError extends Error {
  constructor(readonly code: 'auth_missing' | 'auth_invalid', message: string) {
    super(message)
    this.name = 'ClaudeCodeAuthError'
  }
}

export function parseClaudeAuthData<T extends z.core.$ZodType>(
  schema: T, value: unknown, source: string,
): z.core.output<T> {
  try {
    return parseProviderData(schema, value, source)
  } catch (error) {
    if (error instanceof ProviderDataError) {
      throw new ClaudeCodeAuthError('auth_invalid', errorMessage(error))
    }
    throw error
  }
}

export function parseClaudeAuthJson<T extends z.core.$ZodType>(
  schema: T, text: string, source: string,
): z.core.output<T> {
  try {
    return parseProviderJson(schema, text, source)
  } catch (error) {
    if (error instanceof ProviderDataError) {
      throw new ClaudeCodeAuthError('auth_invalid', errorMessage(error))
    }
    throw error
  }
}

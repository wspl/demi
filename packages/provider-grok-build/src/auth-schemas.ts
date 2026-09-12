import { z } from 'zod'
import { parseProviderData, parseProviderJwt } from '@demicodes/provider'

const token = z.string().min(1).regex(/^\S+$/)
const text = z.string().min(1).regex(/\S/)
const lifetimeSeconds = z.number().int().nonnegative().refine((seconds) =>
  Number.isFinite(new Date(Date.now() + seconds * 1000).getTime()),
)
export const grokIssuerSchema = z.url({ protocol: /^https?$/ })
export const grokAuthEntrySchema = z.looseObject({
  key: token,
  auth_mode: token.optional(),
  refresh_token: token.optional(),
  expires_at: z.iso.datetime({ offset: true }).optional(),
  oidc_issuer: grokIssuerSchema.optional(),
  oidc_client_id: token.optional(),
  email: text.optional(),
  first_name: text.optional(),
  last_name: text.optional(),
  user_id: token.optional(),
  team_id: token.optional(),
  principal_type: token.optional(),
  principal_id: token.optional(),
  organization_id: token.optional(),
})
export const grokAuthFileSchema = z.record(token, grokAuthEntrySchema)
export const grokTokenResponseSchema = z.looseObject({
  access_token: token,
  refresh_token: token.optional(),
  expires_in: lifetimeSeconds.optional(),
  token_type: token.optional(),
  id_token: token.optional(),
})
export const grokClaimsSchema = z.looseObject({
  exp: z.number().int().nonnegative().max(8_640_000_000_000).optional(),
  sub: token.optional(),
  email: text.optional(),
  principal_type: token.optional(),
  principalType: token.optional(),
  principal_id: token.optional(),
  principalId: token.optional(),
  team_id: token.optional(),
})
const verificationUri = z.url().refine((uri) => {
  if (/[\u0000-\u001f]/.test(uri)) {
    return false
  }
  const parsed = new URL(uri)
  return parsed.protocol === 'https:' || (parsed.protocol === 'http:'
    && (parsed.hostname === 'localhost' || parsed.hostname === '127.0.0.1'))
})
export const grokDeviceCodeSchema = z.looseObject({
  device_code: token,
  user_code: z.string().regex(/^[A-Za-z0-9-]+$/),
  verification_uri: verificationUri,
  verification_uri_complete: verificationUri.optional(),
  interval: z.number().int().nonnegative().max(2_147_483).default(5),
  expires_in: lifetimeSeconds.refine((seconds) => seconds > 0),
})
export const grokOAuthErrorSchema = z.looseObject({ error: token })
export const grokUserInfoSchema = z.looseObject({
  userId: token.optional(),
  user_id: token.optional(),
  firstName: text.optional(),
  first_name: text.optional(),
  lastName: text.optional(),
  last_name: text.optional(),
  principalType: token.optional(),
  principal_type: token.optional(),
  principalId: token.optional(),
  principal_id: token.optional(),
  teamId: token.optional(),
  team_id: token.optional(),
  organizationId: token.optional(),
  organization_id: token.optional(),
  email: text.optional(),
}).refine((value) => value.userId !== undefined || value.user_id !== undefined, {
  path: ['userId'], message: 'A user identity is required',
})
export const grokCredentialAddSchema = z.union([
  z.strictObject({ authJsonText: z.string() }),
  z.strictObject({ authFile: text }),
  z.strictObject({ entryKey: token, entry: grokAuthEntrySchema }),
])

export type GrokAuthEntry = z.infer<typeof grokAuthEntrySchema>
export type GrokAuthDotJson = z.infer<typeof grokAuthFileSchema>
export type GrokRefreshTokenResponse = z.infer<typeof grokTokenResponseSchema>
export type GrokUserInfo = z.infer<typeof grokUserInfoSchema>

/** Validate every entry before an import can write any of them. */
export function parseGrokAuthData(value: unknown): GrokAuthDotJson {
  const file = parseProviderData(grokAuthFileSchema, value, 'Grok auth file')
  for (const [entryKey, entry] of Object.entries(file)) {
    parseProviderJwt(grokClaimsSchema, entry.key, 'Grok JWT claims')
    const separator = entryKey.indexOf('::')
    if (!entry.oidc_issuer && separator > 0) {
      parseProviderData(grokIssuerSchema, entryKey.slice(0, separator), 'Grok entry issuer')
    }
  }
  return file
}

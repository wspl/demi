import { z } from 'zod'

const token = z.string().min(1).regex(/^\S+$/)
const label = z.string().min(1).regex(/\S/)
export const codexAuthModeSchema = z.enum([
  'apiKey', 'chatgpt', 'chatgptAuthTokens', 'agentIdentity',
  'personalAccessToken', 'bedrockApiKey',
])
const accountClaimsSchema = z.looseObject({
  chatgpt_account_id: token.optional(),
  chatgpt_account_is_fedramp: z.boolean().optional(),
})
export const codexIdClaimsSchema = accountClaimsSchema.extend({ email: label.optional() })
export const codexJwtClaimsSchema = z.looseObject({
  exp: z.number().int().nonnegative().max(8_640_000_000_000).optional(),
  email: label.optional(),
  'https://api.openai.com/auth': accountClaimsSchema.optional(),
  'https://api.openai.com/profile': z.looseObject({ email: label.optional() }).optional(),
})
export const codexTokenDataSchema = z.looseObject({
  id_token: z.union([token, codexIdClaimsSchema]).optional(),
  access_token: token.optional(),
  refresh_token: token.optional(),
  account_id: token.nullable().optional(),
})
export const codexAuthFileSchema = z.looseObject({
  auth_mode: codexAuthModeSchema.optional(),
  OPENAI_API_KEY: token.nullable().optional(),
  tokens: codexTokenDataSchema.nullable().optional(),
  last_refresh: z.iso.datetime({ offset: true }).nullable().optional(),
  agent_identity: z.looseObject({
    authorization: label,
    account_id: token,
    chatgpt_account_is_fedramp: z.boolean().optional(),
  }).optional(),
  personal_access_token: token.nullable().optional(),
  // Bedrock is recognized only to report an unsupported authentication mode.
  bedrock_api_key: z.unknown().optional(),
})
export const codexRefreshResponseSchema = z.looseObject({
  access_token: token,
  id_token: token.optional(),
  refresh_token: token.optional(),
})
export const codexExchangedTokensSchema = codexRefreshResponseSchema.required()

export const CODEX_DEVICE_LOGIN_MAX_WAIT_MS = 15 * 60 * 1000
const intervalSeconds = z.number().int().nonnegative().max(CODEX_DEVICE_LOGIN_MAX_WAIT_MS / 1000)
export const codexDeviceCodeSchema = z.looseObject({
  device_auth_id: token,
  user_code: token.optional(),
  usercode: token.optional(),
  interval: z.union([
    intervalSeconds,
    z.string().regex(/^\d+$/).pipe(z.coerce.number()).pipe(intervalSeconds),
  ]).default(5),
}).refine((value) => value.user_code !== undefined || value.usercode !== undefined, {
  path: ['user_code'], message: 'A user code is required',
}).refine((value) => value.user_code === undefined || value.usercode === undefined
  || value.user_code === value.usercode, {
  path: ['usercode'], message: 'User codes disagree',
})
export const codexDeviceAuthorizationSchema = z.looseObject({
  authorization_code: token,
  code_verifier: token,
})

export type CodexAuthMode = z.infer<typeof codexAuthModeSchema>
export type CodexTokenData = z.infer<typeof codexTokenDataSchema>
export type CodexAuthDotJson = z.infer<typeof codexAuthFileSchema>
export type RefreshTokenResponse = z.infer<typeof codexRefreshResponseSchema>

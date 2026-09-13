// Native Grok device-code login. Transport is RFC 8628 (Tokens page cannot do
// loopback PKCE); the request contract matches the official Grok CLI: frozen
// OAuth2 scopes, referrer=grok-build, client version/surface headers, id_token
// + access-token principal peek, then cli-chat-proxy GET /user enrichment.
import { delay } from '@demicodes/utils'
import { z } from 'zod'
import {
  GrokAuthError,
  grokJwtClaims,
  peekGrokAccessTokenPrincipal,
  type GrokAuthEntry,
} from './auth'
import {
  DEFAULT_GROK_BUILD_BASE_URL,
  GROK_CLI_TOKEN_AUTH,
  resolveGrokClientVersion
} from './headers'

const GROK_ISSUER = 'https://auth.x.ai'
const GROK_CLI_CLIENT_ID = 'b1a00492-073a-47ea-816f-4c329264a828'
const GROK_LOGIN_SCOPE =
  'openid profile email offline_access grok-cli:access api:access conversations:read conversations:write workspaces:read workspaces:write'
const GROK_LOGIN_REFERRER = 'grok-build'
const GROK_LOGIN_FALLBACK_INTERVAL_S = 5
const GROK_LOGIN_MIN_EXPIRES_S = 10 * 60
const TEAM_PRINCIPAL = 'Team'
const ORGANIZATION_PRINCIPAL = 'Organization'

type GrokLoginSurface = 'ui' | 'cli' | 'headless'

export interface GrokDeviceLoginPending {
  verificationUrl: string
  userCode: string
  expiresAt: string
}

export interface GrokDeviceLoginOptions {
  signal?: AbortSignal
  /** Fires once with the URL + one-time code the user needs. */
  onPending?: (pending: GrokDeviceLoginPending) => void
  fetch?: typeof fetch
  issuer?: string
  clientId?: string
  scope?: string
  surface?: GrokLoginSurface
}

export interface GrokDeviceLoginResult {
  entryKey: string
  entry: GrokAuthEntry
}

function resolveLoginSurface(options: GrokDeviceLoginOptions): GrokLoginSurface {
  if (options.surface)
    return options.surface
  return options.onPending ? 'ui' : 'headless'
}

function oauthHeaders(surface: GrokLoginSurface): Record<string, string> {
  return {
    'content-type': 'application/x-www-form-urlencoded',
    'x-grok-client-version': resolveGrokClientVersion(),
    'x-grok-client-surface': surface,
  }
}

async function postForm(
  fetchImpl: typeof fetch,
  url: string,
  params: Record<string, string>,
  surface: GrokLoginSurface,
  signal?: AbortSignal,
): Promise<Response> {
  return fetchImpl(url, {
    method: 'POST',
    headers: oauthHeaders(surface),
    body: new URLSearchParams(params).toString(),
    signal,
  })
}

/** Decodes one login response; `what` names the step in the failure. */
async function decodeJsonBody<T>(
  response: Response,
  schema: z.ZodType<T>,
  what: string,
): Promise<T> {
  const body: unknown = await response.json().catch(() => null)
  const decoded = schema.safeParse(body)
  if (!decoded.success) {
    throw new GrokAuthError(
      'auth_invalid',
      `${what} response is malformed: ${z.prettifyError(decoded.error)}`
    )
  }
  return decoded.data
}

/**
 * A duration in seconds as the OAuth server states it: RFC 8628 says a number,
 * some deployments send the digits as a string.
 */
const oauthSecondsSchema = z.union([
  z.number(),
  z.string().trim().regex(/^\d+(\.\d+)?$/).transform(Number),
])

/**
 * How long to wait between polls. Anything unusable means "no preference",
 * which is the fallback rather than a failed login.
 */
const pollIntervalSecondsSchema = oauthSecondsSchema
  .refine((seconds) => Number.isFinite(seconds) && seconds >= 0)
  .catch(GROK_LOGIN_FALLBACK_INTERVAL_S)

/** A device-code or token lifetime; an unusable one reads as absent. */
const lifetimeSecondsSchema = oauthSecondsSchema
  .refine((seconds) => Number.isFinite(seconds) && seconds > 0)
  .optional()
  .catch(undefined)

/** The one-time code the user types: letters, digits and dashes only. */
const userCodeSchema = z
  .string()
  .min(1)
  .refine(
    (code) => /^[A-Za-z0-9-]+$/.test(code),
    'must be letters, digits, or dashes'
  )

/**
 * The user is told to open this URL, so only a scheme a browser can be trusted
 * with passes: https anywhere, http on the loopback host. Control characters
 * are rejected before parsing, because `new URL` percent-encodes them into a
 * link that no longer reads as the one the user was shown.
 */
function isBrowsableVerificationUri(uri: string): boolean {
  if ([...uri].some((ch) => ch.charCodeAt(0) < 32))
    return false
  let parsed: URL
  try {
    parsed = new URL(uri)
  } catch {
    return false
  }
  if (parsed.protocol === 'https:')
    return true
  return parsed.protocol === 'http:'
    && (parsed.hostname === 'localhost' || parsed.hostname === '127.0.0.1')
}

const verificationUriSchema = z
  .string()
  .min(1)
  .refine(isBrowsableVerificationUri, 'must be https, or http on localhost')

/** The device-code request's answer: what to show the user and how to poll. */
const deviceCodeResponseSchema = z
  .looseObject({
    device_code: z.string().min(1),
    user_code: userCodeSchema,
    verification_uri: verificationUriSchema.optional(),
    verification_uri_complete: verificationUriSchema.optional(),
    interval: pollIntervalSecondsSchema,
    expires_in: lifetimeSecondsSchema,
  })
  .transform((body, ctx) => {
    const verificationUrl = body.verification_uri_complete
      ?? body.verification_uri
    if (!verificationUrl) {
      ctx.addIssue({
        code: 'custom',
        path: ['verification_uri'],
        message: 'verification_uri is missing',
      })
      return z.NEVER
    }
    return {
      deviceCode: body.device_code,
      userCode: body.user_code,
      verificationUrl,
      intervalSeconds: body.interval,
      expiresInSeconds: body.expires_in,
    }
  })

/** The confirmed login's tokens. */
const deviceTokensSchema = z
  .looseObject({
    access_token: z.string().min(1),
    refresh_token: z.string().min(1).optional(),
    expires_in: lifetimeSecondsSchema,
    id_token: z.string().min(1).optional(),
  })
  .transform((body) => ({
    accessToken: body.access_token,
    refreshToken: body.refresh_token ?? null,
    expiresIn: body.expires_in ?? null,
    idToken: body.id_token ?? null,
  }))

/**
 * A poll the server refused. An unreadable `error` still ends the login, with
 * the HTTP status naming the failure instead.
 */
const deviceTokenErrorSchema = z.looseObject({
  error: z.string().min(1).optional().catch(undefined),
})

type DeviceAuthorization = {
  deviceCode: string
  userCode: string
  verificationUrl: string
  intervalSeconds: number
  expiresAt: number
}

async function requestDeviceCode(
  fetchImpl: typeof fetch,
  issuer: string,
  clientId: string,
  scope: string,
  surface: GrokLoginSurface,
  signal?: AbortSignal,
): Promise<DeviceAuthorization> {
  const response = await postForm(
    fetchImpl,
    `${issuer}/oauth2/device/code`,
    { client_id: clientId, scope, referrer: GROK_LOGIN_REFERRER },
    surface,
    signal,
  )
  if (!response.ok) {
    throw new GrokAuthError(
      'auth_invalid',
      `Grok device code request failed with HTTP ${response.status}`
    )
  }
  const body = await decodeJsonBody(
    response,
    deviceCodeResponseSchema,
    'Grok device code'
  )
  const lifetimeSeconds = Math.max(
    body.expiresInSeconds ?? 0,
    GROK_LOGIN_MIN_EXPIRES_S
  )
  return {
    deviceCode: body.deviceCode,
    userCode: body.userCode,
    verificationUrl: body.verificationUrl,
    intervalSeconds: body.intervalSeconds,
    expiresAt: Date.now() + lifetimeSeconds * 1000,
  }
}

type DeviceTokens = {
  accessToken: string;
  refreshToken: string | null;
  expiresIn: number | null;
  idToken: string | null
}

async function pollForTokens(
  fetchImpl: typeof fetch,
  issuer: string,
  clientId: string,
  device: DeviceAuthorization,
  surface: GrokLoginSurface,
  signal?: AbortSignal,
): Promise<DeviceTokens> {
  let intervalSeconds = Math.max(device.intervalSeconds, 1)
  for (;;) {
    signal?.throwIfAborted()
    await delay(intervalSeconds * 1000)
    if (Date.now() >= device.expiresAt) {
      throw new GrokAuthError(
        'auth_invalid',
        'Grok device login timed out before the user confirmed'
      )
    }
    const response = await postForm(
      fetchImpl,
      `${issuer}/oauth2/token`,
      {
        grant_type: 'urn:ietf:params:oauth:grant-type:device_code',
        device_code: device.deviceCode,
        client_id: clientId
      },
      surface,
      signal,
    )
    if (response.ok)
      return decodeJsonBody(response, deviceTokensSchema, 'Grok device token')
    const { error } = await decodeJsonBody(
      response,
      deviceTokenErrorSchema,
      'Grok device token'
    )
    if (error === 'slow_down') {
      intervalSeconds += 5
    } else if (error !== 'authorization_pending') {
      throw new GrokAuthError(
        'auth_invalid',
        `Grok device login failed: ${error ?? `HTTP ${response.status}`}`
      )
    }
  }
}

/**
 * The cli-chat-proxy `/user` payload, which spells its fields in both camel and
 * snake case. It only fills in display fields of an entry that is already
 * usable, so a field of the wrong type is dropped rather than failing a login
 * the user has already confirmed.
 */
const enrichmentStringSchema = z.string().min(1).optional().catch(undefined)

const grokUserInfoSchema = z.looseObject({
  userId: enrichmentStringSchema,
  user_id: enrichmentStringSchema,
  firstName: enrichmentStringSchema,
  first_name: enrichmentStringSchema,
  lastName: enrichmentStringSchema,
  last_name: enrichmentStringSchema,
  principalType: enrichmentStringSchema,
  principal_type: enrichmentStringSchema,
  principalId: enrichmentStringSchema,
  principal_id: enrichmentStringSchema,
  teamId: enrichmentStringSchema,
  team_id: enrichmentStringSchema,
  organizationId: enrichmentStringSchema,
  organization_id: enrichmentStringSchema,
  email: enrichmentStringSchema,
})

type GrokUserInfo = z.infer<typeof grokUserInfoSchema>

async function fetchUserEnrichment(
  fetchImpl: typeof fetch,
  accessToken: string,
  signal?: AbortSignal,
): Promise<GrokUserInfo | null> {
  try {
    const response = await fetchImpl(`${DEFAULT_GROK_BUILD_BASE_URL}/user`, {
      headers: {
        authorization: `Bearer ${accessToken}`,
        'X-XAI-Token-Auth': GROK_CLI_TOKEN_AUTH,
        'x-grok-client-version': resolveGrokClientVersion(),
        'x-grok-client-mode': 'interactive',
      },
      signal,
    })
    if (!response.ok)
      return null
    const user = grokUserInfoSchema.safeParse(
      await response.json().catch(() => null)
    )
    if (!user.success)
      return null
    // A payload that names no user enriches nothing.
    if (!user.data.userId && !user.data.user_id)
      return null
    return user.data
  } catch {
    return null
  }
}

function applyUserInfoEnrichment(
  entry: GrokAuthEntry,
  user: GrokUserInfo
): void {
  const userId = user.userId ?? user.user_id
  if (userId)
    entry.user_id = userId
  const firstName = user.firstName ?? user.first_name
  if (firstName)
    entry.first_name = firstName
  const lastName = user.lastName ?? user.last_name
  if (lastName)
    entry.last_name = lastName
  const principalType = user.principalType ?? user.principal_type
  if (principalType)
    entry.principal_type = principalType
  const principalId = user.principalId ?? user.principal_id
  if (principalId)
    entry.principal_id = principalId
  const teamId = user.teamId ?? user.team_id
  if (teamId)
    entry.team_id = teamId
  const organizationId = user.organizationId ?? user.organization_id
  if (organizationId)
    entry.organization_id = organizationId
  if (user.email)
    entry.email = user.email
}

async function assembleAuthEntry(
  fetchImpl: typeof fetch,
  issuer: string,
  clientId: string,
  tokens: DeviceTokens,
  signal?: AbortSignal,
): Promise<GrokAuthEntry> {
  const idClaims = tokens.idToken ? grokJwtClaims(tokens.idToken) : null
  let userId = idClaims?.sub ?? ''
  let email = idClaims?.email ?? null
  const peeked = peekGrokAccessTokenPrincipal(tokens.accessToken)
  const principalType = peeked?.principalType ?? null
  const principalId = peeked?.principalId ?? null
  let teamId = peeked?.teamId ?? null
  let organizationId: string | null = null

  if (principalType === TEAM_PRINCIPAL && principalId) {
    userId = principalId
    email = null
    teamId = principalId
  } else if (principalType === ORGANIZATION_PRINCIPAL && principalId) {
    userId = principalId
    email = null
    organizationId = principalId
  }

  const entry: GrokAuthEntry = {
    key: tokens.accessToken,
    auth_mode: 'oidc',
    ...(tokens.refreshToken ? { refresh_token: tokens.refreshToken } : {}),
    ...(tokens.expiresIn ? {
      expires_at: new Date(Date.now() + tokens.expiresIn * 1000).toISOString()
    } : {}),
    oidc_issuer: issuer,
    oidc_client_id: clientId,
    ...(email ? { email } : {}),
    ...(userId ? { user_id: userId } : {}),
    ...(principalType ? { principal_type: principalType } : {}),
    ...(principalId ? { principal_id: principalId } : {}),
    ...(teamId ? { team_id: teamId } : {}),
    ...(organizationId ? { organization_id: organizationId } : {}),
  }

  const enriched = await fetchUserEnrichment(
    fetchImpl,
    tokens.accessToken,
    signal
  )
  if (enriched)
    applyUserInfoEnrichment(entry, enriched)
  return entry
}

/**
 * Runs the full device flow and returns a vendor-shaped auth.json entry keyed
 * like the Grok CLI.
 */
export async function runGrokDeviceLogin(
  options: GrokDeviceLoginOptions = {}
): Promise<GrokDeviceLoginResult> {
  const fetchImpl = options.fetch ?? fetch
  const issuer = (options.issuer ?? GROK_ISSUER).replace(/\/+$/, '')
  const clientId = options.clientId ?? GROK_CLI_CLIENT_ID
  const scope = options.scope ?? GROK_LOGIN_SCOPE
  const surface = resolveLoginSurface(options)

  const device = await requestDeviceCode(
    fetchImpl,
    issuer,
    clientId,
    scope,
    surface,
    options.signal
  )
  options.onPending?.({
    verificationUrl: device.verificationUrl,
    userCode: device.userCode,
    expiresAt: new Date(device.expiresAt).toISOString(),
  })

  const tokens = await pollForTokens(
    fetchImpl,
    issuer,
    clientId,
    device,
    surface,
    options.signal
  )
  const entry = await assembleAuthEntry(
    fetchImpl,
    issuer,
    clientId,
    tokens,
    options.signal
  )
  return { entryKey: `${issuer}::${clientId}`, entry }
}

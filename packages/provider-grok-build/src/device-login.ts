// Native Grok device-code login. Transport is RFC 8628 (Tokens page cannot do
// loopback PKCE); the request contract matches the official Grok CLI: frozen
// OAuth2 scopes, referrer=grok-build, client version/surface headers, id_token
// + access-token principal peek, then cli-chat-proxy GET /user enrichment.
import { delay } from '@demicodes/utils'
import { parseProviderJson } from '@demicodes/provider'
import {
  grokDeviceCodeSchema, grokTokenResponseSchema, grokOAuthErrorSchema, grokUserInfoSchema,
  type GrokUserInfo,
} from './auth-schemas'
import {
  GrokAuthError,
  decodeGrokJwtPayload,
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
const TEAM_PRINCIPAL = 'Team'
const ORGANIZATION_PRINCIPAL = 'Organization'

type GrokLoginSurface = 'ui' | 'cli' | 'headless'

export interface GrokDeviceLoginPending {
  verificationUrl: string
  userCode: string
  expiresAt: string
}

export interface GrokDeviceLoginOptions {
  clientVersion?: string
  grokHome?: string
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

function oauthHeaders(surface: GrokLoginSurface, clientVersion: string): Record<string, string> {
  return {
    'content-type': 'application/x-www-form-urlencoded',
    'x-grok-client-version': clientVersion,
    'x-grok-client-surface': surface,
  }
}

async function postForm(
  fetchImpl: typeof fetch,
  url: string,
  params: Record<string, string>,
  surface: GrokLoginSurface,
  clientVersion: string,
  signal?: AbortSignal,
): Promise<Response> {
  return fetchImpl(url, {
    method: 'POST',
    headers: oauthHeaders(surface, clientVersion),
    body: new URLSearchParams(params).toString(),
    signal,
  })
}

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
  clientVersion: string,
  signal?: AbortSignal,
): Promise<DeviceAuthorization> {
  const response = await postForm(
    fetchImpl,
    `${issuer}/oauth2/device/code`,
    { client_id: clientId, scope, referrer: GROK_LOGIN_REFERRER },
    surface,
    clientVersion,
    signal,
  )
  if (!response.ok) {
    throw new GrokAuthError(
      'auth_invalid',
      `Grok device code request failed with HTTP ${response.status}`
    )
  }
  const body = parseProviderJson(grokDeviceCodeSchema, await response.text(), 'Grok device code')
  return {
    deviceCode: body.device_code,
    userCode: body.user_code,
    verificationUrl: body.verification_uri_complete ?? body.verification_uri,
    intervalSeconds: body.interval,
    expiresAt: Date.now() + body.expires_in * 1000,
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
  clientVersion: string,
  signal?: AbortSignal,
): Promise<DeviceTokens> {
  let intervalSeconds = Math.max(device.intervalSeconds, 1)
  for (;;) {
    signal?.throwIfAborted()
    const remainingMs = Math.max(0, device.expiresAt - Date.now())
    await delay(Math.min(intervalSeconds * 1000, remainingMs), signal)
    signal?.throwIfAborted()
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
      clientVersion,
      signal,
    )
    if (response.ok) {
      const body = parseProviderJson(grokTokenResponseSchema, await response.text(), 'Grok device token')
      return {
        accessToken: body.access_token,
        refreshToken: body.refresh_token ?? null,
        expiresIn: body.expires_in ?? null,
        idToken: body.id_token ?? null,
      }
    }
    const { error } = parseProviderJson(grokOAuthErrorSchema, await response.text(), 'Grok OAuth error')
    if (error === 'slow_down') {
      intervalSeconds = Math.min(intervalSeconds + 5, 2_147_483)
    } else if (error !== 'authorization_pending') {
      throw new GrokAuthError(
        'auth_invalid',
        `Grok device login failed: ${error}`
      )
    }
  }
}

async function fetchUserEnrichment(
  fetchImpl: typeof fetch,
  accessToken: string,
  clientVersion: string,
  signal?: AbortSignal,
): Promise<GrokUserInfo | null> {
  let response: Response
  try {
    response = await fetchImpl(`${DEFAULT_GROK_BUILD_BASE_URL}/user`, {
      headers: {
        authorization: `Bearer ${accessToken}`,
        'X-XAI-Token-Auth': GROK_CLI_TOKEN_AUTH,
        'x-grok-client-version': clientVersion,
        'x-grok-client-mode': 'interactive',
      },
      signal,
    })
  } catch (error) {
    signal?.throwIfAborted()
    // User profile enrichment is optional when its endpoint is unavailable.
    if (error instanceof TypeError) {
      return null
    }
    throw error
  }
  signal?.throwIfAborted()
  if (!response.ok) {
    return null
  }
  return parseProviderJson(grokUserInfoSchema, await response.text(), 'Grok user profile')
}

function applyUserInfoEnrichment(
  entry: GrokAuthEntry,
  user: GrokUserInfo
): void {
  const userId = user.userId ?? user.user_id
  if (userId)
    entry.user_id = userId
  const firstName = user.firstName
    ?? user.first_name
  if (firstName)
    entry.first_name = firstName
  const lastName = user.lastName
    ?? user.last_name
  if (lastName)
    entry.last_name = lastName
  const principalType = user.principalType
    ?? user.principal_type
  if (principalType)
    entry.principal_type = principalType
  const principalId = user.principalId
    ?? user.principal_id
  if (principalId)
    entry.principal_id = principalId
  const teamId = user.teamId ?? user.team_id
  if (teamId)
    entry.team_id = teamId
  const organizationId = user.organizationId
    ?? user.organization_id
  if (organizationId)
    entry.organization_id = organizationId
  const email = user.email
  if (email)
    entry.email = email
}

async function assembleAuthEntry(
  fetchImpl: typeof fetch,
  issuer: string,
  clientId: string,
  tokens: DeviceTokens,
  clientVersion: string,
  signal?: AbortSignal,
): Promise<GrokAuthEntry> {
  const idClaims = tokens.idToken ? decodeGrokJwtPayload(tokens.idToken) : null
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
    ...(tokens.expiresIn !== null ? {
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
    clientVersion,
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
  const clientVersion = resolveGrokClientVersion(options.clientVersion, options.grokHome)

  const device = await requestDeviceCode(
    fetchImpl,
    issuer,
    clientId,
    scope,
    surface,
    clientVersion,
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
    clientVersion,
    options.signal
  )
  const entry = await assembleAuthEntry(
    fetchImpl,
    issuer,
    clientId,
    tokens,
    clientVersion,
    options.signal
  )
  return { entryKey: `${issuer}::${clientId}`, entry }
}

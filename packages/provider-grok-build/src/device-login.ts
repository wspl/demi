// Native Grok device-code login. Transport is RFC 8628 (Tokens page cannot do
// loopback PKCE); the request contract matches the official Grok CLI: frozen
// OAuth2 scopes, referrer=grok-build, client version/surface headers, id_token
// + access-token principal peek, then cli-chat-proxy GET /user enrichment.
import { delay, isRecord, nonEmptyString } from '@demicodes/utils'
import {
  GrokAuthError,
  decodeGrokJwtPayload,
  peekGrokAccessTokenPrincipal,
  type GrokAuthEntry,
} from './auth'
import { DEFAULT_GROK_BUILD_BASE_URL, GROK_CLI_TOKEN_AUTH, resolveGrokClientVersion } from './headers'

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
  if (options.surface) return options.surface
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

async function jsonBody(response: Response, what: string): Promise<Record<string, unknown>> {
  const body: unknown = await response.json().catch(() => null)
  if (!isRecord(body)) throw new GrokAuthError('auth_invalid', `${what} response is not a JSON object`)
  return body
}

function assertUserCode(userCode: string): void {
  if (![...userCode].every((ch) => /[A-Za-z0-9-]/.test(ch))) {
    throw new GrokAuthError('auth_invalid', 'Grok device code response has an invalid user_code')
  }
}

function assertVerificationUri(uri: string): void {
  if ([...uri].some((ch) => ch.charCodeAt(0) < 32)) {
    throw new GrokAuthError('auth_invalid', 'Grok device code response has an invalid verification URI')
  }
  let parsed: URL
  try {
    parsed = new URL(uri)
  } catch {
    throw new GrokAuthError('auth_invalid', 'Grok device code response has an invalid verification URI')
  }
  if (parsed.protocol === 'https:') return
  if (parsed.protocol === 'http:' && (parsed.hostname === 'localhost' || parsed.hostname === '127.0.0.1')) return
  throw new GrokAuthError('auth_invalid', 'Grok device code response has an unsupported verification URI scheme')
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
    throw new GrokAuthError('auth_invalid', `Grok device code request failed with HTTP ${response.status}`)
  }
  const body = await jsonBody(response, 'Grok device code')
  const deviceCode = nonEmptyString(body.device_code)
  const userCode = nonEmptyString(body.user_code)
  const verificationUrl = nonEmptyString(body.verification_uri_complete) ?? nonEmptyString(body.verification_uri)
  const verificationUri = nonEmptyString(body.verification_uri)
  if (!deviceCode || !userCode || !verificationUrl) {
    throw new GrokAuthError('auth_invalid', 'Grok device code response is missing device_code, user_code, or verification_uri')
  }
  assertUserCode(userCode)
  if (verificationUri) assertVerificationUri(verificationUri)
  if (nonEmptyString(body.verification_uri_complete)) assertVerificationUri(nonEmptyString(body.verification_uri_complete)!)
  const interval = Number(body.interval)
  const expiresIn = Number(body.expires_in)
  return {
    deviceCode,
    userCode,
    verificationUrl,
    intervalSeconds: Number.isFinite(interval) && interval >= 0 ? interval : GROK_LOGIN_FALLBACK_INTERVAL_S,
    expiresAt:
      Date.now() + Math.max(Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : 0, GROK_LOGIN_MIN_EXPIRES_S) * 1000,
  }
}

type DeviceTokens = { accessToken: string; refreshToken: string | null; expiresIn: number | null; idToken: string | null }

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
      throw new GrokAuthError('auth_invalid', 'Grok device login timed out before the user confirmed')
    }
    const response = await postForm(
      fetchImpl,
      `${issuer}/oauth2/token`,
      { grant_type: 'urn:ietf:params:oauth:grant-type:device_code', device_code: device.deviceCode, client_id: clientId },
      surface,
      signal,
    )
    const body = await jsonBody(response, 'Grok device token')
    if (response.ok) {
      const accessToken = nonEmptyString(body.access_token)
      if (!accessToken) throw new GrokAuthError('auth_invalid', 'Grok token response is missing access_token')
      const expiresIn = Number(body.expires_in)
      return {
        accessToken,
        refreshToken: nonEmptyString(body.refresh_token) ?? null,
        expiresIn: Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : null,
        idToken: nonEmptyString(body.id_token) ?? null,
      }
    }
    const error = nonEmptyString(body.error)
    if (error === 'slow_down') {
      intervalSeconds += 5
    } else if (error !== 'authorization_pending') {
      throw new GrokAuthError('auth_invalid', `Grok device login failed: ${error ?? `HTTP ${response.status}`}`)
    }
  }
}

async function fetchUserEnrichment(
  fetchImpl: typeof fetch,
  accessToken: string,
  signal?: AbortSignal,
): Promise<Record<string, unknown> | null> {
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
    if (!response.ok) return null
    const body: unknown = await response.json().catch(() => null)
    if (!isRecord(body)) return null
    if (!nonEmptyString(body.userId) && !nonEmptyString(body.user_id)) return null
    return body
  } catch {
    return null
  }
}

function applyUserInfoEnrichment(entry: GrokAuthEntry, user: Record<string, unknown>): void {
  const userId = nonEmptyString(user.userId) ?? nonEmptyString(user.user_id)
  if (userId) entry.user_id = userId
  const firstName = nonEmptyString(user.firstName) ?? nonEmptyString(user.first_name)
  if (firstName) entry.first_name = firstName
  const lastName = nonEmptyString(user.lastName) ?? nonEmptyString(user.last_name)
  if (lastName) entry.last_name = lastName
  const principalType = nonEmptyString(user.principalType) ?? nonEmptyString(user.principal_type)
  if (principalType) entry.principal_type = principalType
  const principalId = nonEmptyString(user.principalId) ?? nonEmptyString(user.principal_id)
  if (principalId) entry.principal_id = principalId
  const teamId = nonEmptyString(user.teamId) ?? nonEmptyString(user.team_id)
  if (teamId) entry.team_id = teamId
  const organizationId = nonEmptyString(user.organizationId) ?? nonEmptyString(user.organization_id)
  if (organizationId) entry.organization_id = organizationId
  const email = nonEmptyString(user.email)
  if (email) entry.email = email
}

async function assembleAuthEntry(
  fetchImpl: typeof fetch,
  issuer: string,
  clientId: string,
  tokens: DeviceTokens,
  signal?: AbortSignal,
): Promise<GrokAuthEntry> {
  const idClaims = tokens.idToken ? decodeGrokJwtPayload(tokens.idToken) : null
  let userId = nonEmptyString(idClaims?.sub) ?? ''
  let email = nonEmptyString(idClaims?.email) ?? null
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
    ...(tokens.expiresIn ? { expires_at: new Date(Date.now() + tokens.expiresIn * 1000).toISOString() } : {}),
    oidc_issuer: issuer,
    oidc_client_id: clientId,
    ...(email ? { email } : {}),
    ...(userId ? { user_id: userId } : {}),
    ...(principalType ? { principal_type: principalType } : {}),
    ...(principalId ? { principal_id: principalId } : {}),
    ...(teamId ? { team_id: teamId } : {}),
    ...(organizationId ? { organization_id: organizationId } : {}),
  }

  const enriched = await fetchUserEnrichment(fetchImpl, tokens.accessToken, signal)
  if (enriched) applyUserInfoEnrichment(entry, enriched)
  return entry
}

/** Runs the full device flow and returns a vendor-shaped auth.json entry keyed like the Grok CLI. */
export async function runGrokDeviceLogin(options: GrokDeviceLoginOptions = {}): Promise<GrokDeviceLoginResult> {
  const fetchImpl = options.fetch ?? fetch
  const issuer = (options.issuer ?? GROK_ISSUER).replace(/\/+$/, '')
  const clientId = options.clientId ?? GROK_CLI_CLIENT_ID
  const scope = options.scope ?? GROK_LOGIN_SCOPE
  const surface = resolveLoginSurface(options)

  const device = await requestDeviceCode(fetchImpl, issuer, clientId, scope, surface, options.signal)
  options.onPending?.({
    verificationUrl: device.verificationUrl,
    userCode: device.userCode,
    expiresAt: new Date(device.expiresAt).toISOString(),
  })

  const tokens = await pollForTokens(fetchImpl, issuer, clientId, device, surface, options.signal)
  const entry = await assembleAuthEntry(fetchImpl, issuer, clientId, tokens, options.signal)
  return { entryKey: `${issuer}::${clientId}`, entry }
}

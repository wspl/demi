// Native Grok device-code login. auth.x.ai is a standard OIDC issuer that advertises the
// RFC 8628 device authorization grant (see /.well-known/openid-configuration), so demi drives
// it directly: request a device code, let the user confirm from any browser on any device,
// poll the token endpoint, then read userinfo to label the entry. No vendor CLI involved.
import { delay, isRecord, nonEmptyString } from '@demicodes/utils'
import { GrokAuthError, type GrokAuthEntry } from './auth'

const GROK_ISSUER = 'https://auth.x.ai'
// Public OIDC client the Grok CLI registers its ~/.grok/auth.json entries under.
const GROK_CLI_CLIENT_ID = 'b1a00492-073a-47ea-816f-4c329264a828'
const GROK_LOGIN_SCOPE = 'openid profile email offline_access grok-cli:access'
const GROK_LOGIN_FALLBACK_INTERVAL_S = 5
const GROK_LOGIN_FALLBACK_EXPIRES_S = 15 * 60

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
}

export interface GrokDeviceLoginResult {
  entryKey: string
  entry: GrokAuthEntry
}

async function postForm(fetchImpl: typeof fetch, url: string, params: Record<string, string>, signal?: AbortSignal): Promise<Response> {
  return fetchImpl(url, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams(params).toString(),
    signal,
  })
}

async function jsonBody(response: Response, what: string): Promise<Record<string, unknown>> {
  const body: unknown = await response.json().catch(() => null)
  if (!isRecord(body)) throw new GrokAuthError('auth_invalid', `${what} response is not a JSON object`)
  return body
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
  signal?: AbortSignal,
): Promise<DeviceAuthorization> {
  const response = await postForm(fetchImpl, `${issuer}/oauth2/device/code`, { client_id: clientId, scope }, signal)
  if (!response.ok) {
    throw new GrokAuthError('auth_invalid', `Grok device code request failed with HTTP ${response.status}`)
  }
  const body = await jsonBody(response, 'Grok device code')
  const deviceCode = nonEmptyString(body.device_code)
  const userCode = nonEmptyString(body.user_code)
  const verificationUrl = nonEmptyString(body.verification_uri_complete) ?? nonEmptyString(body.verification_uri)
  if (!deviceCode || !userCode || !verificationUrl) {
    throw new GrokAuthError('auth_invalid', 'Grok device code response is missing device_code, user_code, or verification_uri')
  }
  const interval = Number(body.interval)
  const expiresIn = Number(body.expires_in)
  return {
    deviceCode,
    userCode,
    verificationUrl,
    intervalSeconds: Number.isFinite(interval) && interval >= 0 ? interval : GROK_LOGIN_FALLBACK_INTERVAL_S,
    expiresAt: Date.now() + (Number.isFinite(expiresIn) && expiresIn > 0 ? expiresIn : GROK_LOGIN_FALLBACK_EXPIRES_S) * 1000,
  }
}

type DeviceTokens = { accessToken: string; refreshToken: string | null; expiresIn: number | null }

async function pollForTokens(
  fetchImpl: typeof fetch,
  issuer: string,
  clientId: string,
  device: DeviceAuthorization,
  signal?: AbortSignal,
): Promise<DeviceTokens> {
  let intervalSeconds = device.intervalSeconds
  for (;;) {
    signal?.throwIfAborted()
    const response = await postForm(
      fetchImpl,
      `${issuer}/oauth2/token`,
      { grant_type: 'urn:ietf:params:oauth:grant-type:device_code', device_code: device.deviceCode, client_id: clientId },
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
      }
    }
    const error = nonEmptyString(body.error)
    if (error === 'slow_down') {
      intervalSeconds += 5
    } else if (error !== 'authorization_pending') {
      throw new GrokAuthError('auth_invalid', `Grok device login failed: ${error ?? `HTTP ${response.status}`}`)
    }
    if (Date.now() >= device.expiresAt) {
      throw new GrokAuthError('auth_invalid', 'Grok device login timed out before the user confirmed')
    }
    await delay(intervalSeconds * 1000)
  }
}

async function fetchUserinfo(
  fetchImpl: typeof fetch,
  issuer: string,
  accessToken: string,
  signal?: AbortSignal,
): Promise<Record<string, unknown>> {
  const response = await fetchImpl(`${issuer}/oauth2/userinfo`, {
    headers: { authorization: `Bearer ${accessToken}` },
    signal,
  })
  if (!response.ok) return {}
  const body: unknown = await response.json().catch(() => null)
  return isRecord(body) ? body : {}
}

/** Runs the full device flow and returns a vendor-shaped auth.json entry keyed like the Grok CLI. */
export async function runGrokDeviceLogin(options: GrokDeviceLoginOptions = {}): Promise<GrokDeviceLoginResult> {
  const fetchImpl = options.fetch ?? fetch
  const issuer = (options.issuer ?? GROK_ISSUER).replace(/\/+$/, '')
  const clientId = options.clientId ?? GROK_CLI_CLIENT_ID
  const scope = options.scope ?? GROK_LOGIN_SCOPE

  const device = await requestDeviceCode(fetchImpl, issuer, clientId, scope, options.signal)
  options.onPending?.({
    verificationUrl: device.verificationUrl,
    userCode: device.userCode,
    expiresAt: new Date(device.expiresAt).toISOString(),
  })

  const tokens = await pollForTokens(fetchImpl, issuer, clientId, device, options.signal)
  const userinfo = await fetchUserinfo(fetchImpl, issuer, tokens.accessToken, options.signal)

  const entry: GrokAuthEntry = {
    key: tokens.accessToken,
    auth_mode: 'oidc',
    ...(tokens.refreshToken ? { refresh_token: tokens.refreshToken } : {}),
    ...(tokens.expiresIn ? { expires_at: new Date(Date.now() + tokens.expiresIn * 1000).toISOString() } : {}),
    oidc_issuer: issuer,
    oidc_client_id: clientId,
    ...(nonEmptyString(userinfo.email) ? { email: nonEmptyString(userinfo.email)! } : {}),
    ...(nonEmptyString(userinfo.given_name) ? { first_name: nonEmptyString(userinfo.given_name)! } : {}),
    ...(nonEmptyString(userinfo.sub) ? { user_id: nonEmptyString(userinfo.sub)! } : {}),
  }
  return { entryKey: `${issuer}::${clientId}`, entry }
}

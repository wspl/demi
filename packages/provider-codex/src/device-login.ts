// Native ChatGPT device-code login. Protocol mirrors codex-rs `login/src/device_code_auth.rs`:
// request a user code, let the user confirm at {issuer}/codex/device from any browser on any
// device, poll until the server issues an authorization code with server-generated PKCE, then
// run the standard authorization_code exchange. No vendor CLI and no host-side browser involved.
import { delay } from '@demicodes/utils'
import { parseProviderJson } from '@demicodes/provider'
import {
  CODEX_DEVICE_LOGIN_MAX_WAIT_MS,
  codexDeviceAuthorizationSchema,
  codexDeviceCodeSchema,
  codexExchangedTokensSchema,
} from './auth-schemas'
import {
  CodexAuthError,
  codexOauthClientId,
  parseChatGptClaims,
  parseIdTokenClaims,
  type CodexAuthDotJson,
} from './auth'

const DEVICE_LOGIN_ISSUER = 'https://auth.openai.com'

export interface CodexDeviceLoginPending {
  verificationUrl: string
  userCode: string
  expiresAt: string
}

export interface CodexDeviceLoginOptions {
  signal?: AbortSignal
  /** Fires once with the URL + one-time code the user needs. */
  onPending?: (pending: CodexDeviceLoginPending) => void
  fetch?: typeof fetch
  issuer?: string
}

type DeviceUserCode = {
  deviceAuthId: string;
  userCode: string;
  intervalSeconds: number
}
type DeviceAuthorization = {
  authorizationCode: string;
  codeVerifier: string
}
type ExchangedTokens = {
  idToken: string;
  accessToken: string;
  refreshToken: string
}

async function postJson(
  fetchImpl: typeof fetch,
  url: string,
  body: unknown,
  signal?: AbortSignal
): Promise<Response> {
  return fetchImpl(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
    signal,
  })
}

async function requestUserCode(
  fetchImpl: typeof fetch,
  issuer: string,
  clientId: string,
  signal?: AbortSignal
): Promise<DeviceUserCode> {
  const response = await postJson(
    fetchImpl,
    `${issuer}/api/accounts/deviceauth/usercode`,
    { client_id: clientId },
    signal
  )
  if (response.status === 404) {
    throw new CodexAuthError(
      'auth_unsupported',
      'Device-code login is not enabled for this Codex account'
    )
  }
  if (!response.ok) {
    throw new CodexAuthError(
      'auth_login_failed',
      `Device code request failed with HTTP ${response.status}`
    )
  }
  const body = parseProviderJson(codexDeviceCodeSchema, await response.text(), 'Codex device code')
  return {
    deviceAuthId: body.device_auth_id,
    // The schema requires one code and rejects conflicting aliases.
    userCode: body.user_code ?? body.usercode!,
    intervalSeconds: body.interval,
  }
}

async function pollForAuthorization(
  fetchImpl: typeof fetch,
  issuer: string,
  userCode: DeviceUserCode,
  startedAt: number,
  signal?: AbortSignal,
): Promise<DeviceAuthorization> {
  for (;;) {
    signal?.throwIfAborted()
    if (Date.now() - startedAt >= CODEX_DEVICE_LOGIN_MAX_WAIT_MS) {
      throw new CodexAuthError('auth_login_failed', 'Device-code login timed out after 15 minutes')
    }
    const response = await postJson(
      fetchImpl,
      `${issuer}/api/accounts/deviceauth/token`,
      { device_auth_id: userCode.deviceAuthId, user_code: userCode.userCode },
      signal,
    )
    if (response.ok) {
      const body = parseProviderJson(
        codexDeviceAuthorizationSchema, await response.text(), 'Codex device authorization',
      )
      return { authorizationCode: body.authorization_code, codeVerifier: body.code_verifier }
    }
    // 403/404 mean "user has not confirmed yet"; anything else is terminal.
    if (response.status !== 403 && response.status !== 404) {
      throw new CodexAuthError(
        'auth_login_failed',
        `Device authorization failed with HTTP ${response.status}`
      )
    }
    const remainingMs = Math.max(0, CODEX_DEVICE_LOGIN_MAX_WAIT_MS - (Date.now() - startedAt))
    await delay(Math.min(userCode.intervalSeconds * 1000, remainingMs), signal)
  }
}

async function exchangeAuthorizationCode(
  fetchImpl: typeof fetch,
  issuer: string,
  clientId: string,
  authorization: DeviceAuthorization,
  signal?: AbortSignal,
): Promise<ExchangedTokens> {
  const params = new URLSearchParams({
    grant_type: 'authorization_code',
    code: authorization.authorizationCode,
    redirect_uri: `${issuer}/deviceauth/callback`,
    client_id: clientId,
    code_verifier: authorization.codeVerifier,
  })
  const response = await fetchImpl(`${issuer}/oauth/token`, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: params.toString(),
    signal,
  })
  if (!response.ok) {
    throw new CodexAuthError(
      'auth_login_failed',
      `Device-code token exchange failed with HTTP ${response.status}`
    )
  }
  const body = parseProviderJson(codexExchangedTokensSchema, await response.text(), 'Codex token exchange')
  return { idToken: body.id_token, accessToken: body.access_token, refreshToken: body.refresh_token }
}

/** Runs the full device-code flow and returns vendor-shaped auth material. */
export async function runCodexDeviceLogin(
  options: CodexDeviceLoginOptions = {}
): Promise<CodexAuthDotJson> {
  const fetchImpl = options.fetch ?? fetch
  const issuer = (options.issuer ?? DEVICE_LOGIN_ISSUER).replace(/\/+$/, '')
  const clientId = codexOauthClientId()
  const startedAt = Date.now()

  const userCode = await requestUserCode(
    fetchImpl,
    issuer,
    clientId,
    options.signal
  )
  options.onPending?.({
    verificationUrl: `${issuer}/codex/device`,
    userCode: userCode.userCode,
    expiresAt: new Date(startedAt + CODEX_DEVICE_LOGIN_MAX_WAIT_MS).toISOString(),
  })

  const authorization = await pollForAuthorization(
    fetchImpl,
    issuer,
    userCode,
    startedAt,
    options.signal
  )
  const tokens = await exchangeAuthorizationCode(
    fetchImpl,
    issuer,
    clientId,
    authorization,
    options.signal
  )

  const accountId = parseChatGptClaims(tokens.accessToken).accountId
    ?? parseIdTokenClaims(tokens.idToken).accountId
  if (!accountId) {
    throw new CodexAuthError('auth_login_failed', 'Device login tokens have no ChatGPT account ID')
  }
  return {
    auth_mode: 'chatgpt',
    OPENAI_API_KEY: null,
    tokens: {
      id_token: tokens.idToken,
      access_token: tokens.accessToken,
      refresh_token: tokens.refreshToken,
      account_id: accountId,
    },
    last_refresh: new Date().toISOString(),
  }
}

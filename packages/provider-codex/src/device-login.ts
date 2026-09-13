// Native ChatGPT device-code login. Protocol mirrors codex-rs `login/src/device_code_auth.rs`:
// request a user code, let the user confirm at {issuer}/codex/device from any browser on any
// device, poll until the server issues an authorization code with server-generated PKCE, then
// run the standard authorization_code exchange. No vendor CLI and no host-side browser involved.
import { delay } from '@demicodes/utils'
import { z } from 'zod'
import {
  CodexAuthError,
  codexOauthClientId,
  parseChatGptClaims,
  parseIdTokenClaims,
  type CodexAuthDotJson,
} from './auth'

const DEVICE_LOGIN_ISSUER = 'https://auth.openai.com'
const DEVICE_LOGIN_MAX_WAIT_MS = 15 * 60 * 1000
const DEVICE_LOGIN_FALLBACK_INTERVAL_S = 5

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

/**
 * How long to wait between polls, in seconds. The server sends a number, some
 * deployments send it as a digit string; anything else means "no preference",
 * which is the fallback rather than a failed login.
 */
const pollIntervalSecondsSchema = z
  .union([
    z.number(),
    z.string().trim().regex(/^\d+(\.\d+)?$/).transform(Number),
  ])
  .refine((seconds) => Number.isFinite(seconds) && seconds >= 0)
  .catch(DEVICE_LOGIN_FALLBACK_INTERVAL_S)

/** The device-code request's answer: what to show the user and how to poll. */
const deviceUserCodeSchema = z
  .looseObject({
    device_auth_id: z.string().min(1),
    // Deployments disagree on the spelling of this one field.
    user_code: z.string().min(1).optional(),
    usercode: z.string().min(1).optional(),
    interval: pollIntervalSecondsSchema,
  })
  .transform((body, ctx) => {
    const userCode = body.user_code ?? body.usercode
    if (!userCode) {
      ctx.addIssue({ code: 'custom', message: 'user_code is missing' })
      return z.NEVER
    }
    return {
      deviceAuthId: body.device_auth_id,
      userCode,
      intervalSeconds: body.interval,
    }
  })

type DeviceUserCode = z.infer<typeof deviceUserCodeSchema>

/** The confirmed login: an authorization code with server-generated PKCE. */
const deviceAuthorizationSchema = z
  .looseObject({
    authorization_code: z.string().min(1),
    code_verifier: z.string().min(1),
  })
  .transform((body) => ({
    authorizationCode: body.authorization_code,
    codeVerifier: body.code_verifier,
  }))

type DeviceAuthorization = z.infer<typeof deviceAuthorizationSchema>

const exchangedTokensSchema = z
  .looseObject({
    id_token: z.string().min(1),
    access_token: z.string().min(1),
    refresh_token: z.string().min(1),
  })
  .transform((body) => ({
    idToken: body.id_token,
    accessToken: body.access_token,
    refreshToken: body.refresh_token,
  }))

type ExchangedTokens = z.infer<typeof exchangedTokensSchema>

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

/** Decodes one login response; `what` names the step in the failure. */
async function decodeJsonBody<T>(
  response: Response,
  schema: z.ZodType<T>,
  what: string,
): Promise<T> {
  const body: unknown = await response.json().catch(() => null)
  const decoded = schema.safeParse(body)
  if (!decoded.success) {
    throw new CodexAuthError(
      'auth_login_failed',
      `${what} response is malformed: ${z.prettifyError(decoded.error)}`
    )
  }
  return decoded.data
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
  return decodeJsonBody(response, deviceUserCodeSchema, 'Device code')
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
    const response = await postJson(
      fetchImpl,
      `${issuer}/api/accounts/deviceauth/token`,
      { device_auth_id: userCode.deviceAuthId, user_code: userCode.userCode },
      signal,
    )
    if (response.ok) {
      return decodeJsonBody(
        response,
        deviceAuthorizationSchema,
        'Device authorization'
      )
    }
    // 403/404 mean "user has not confirmed yet"; anything else is terminal.
    if (response.status !== 403 && response.status !== 404) {
      throw new CodexAuthError(
        'auth_login_failed',
        `Device authorization failed with HTTP ${response.status}`
      )
    }
    if (Date.now() - startedAt >= DEVICE_LOGIN_MAX_WAIT_MS) {
      throw new CodexAuthError(
        'auth_login_failed',
        'Device-code login timed out after 15 minutes'
      )
    }
    await delay(userCode.intervalSeconds * 1000)
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
  return decodeJsonBody(response, exchangedTokensSchema, 'Token exchange')
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
    expiresAt: new Date(startedAt + DEVICE_LOGIN_MAX_WAIT_MS).toISOString(),
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

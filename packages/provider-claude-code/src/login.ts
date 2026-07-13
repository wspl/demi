// Claude Pro/Max OAuth login with manual code copy-back. The user opens the authorize URL
// from any browser on any device; after approval the vendor callback page displays a
// "code#state" string the user brings back (collected via promptForCode). PKCE S256; the
// token endpoint also serves refresh_token grants for pool-entry renewal.
// Constants verified against the shipped Claude Code CLI binary (client id, authorize URL,
// scopes, /v1/oauth/token and /oauth/code/callback paths).
import { randomBytes, createHash } from 'node:crypto'
import { isRecord, nonEmptyString } from '@demicodes/utils'
import { ClaudeCodeAuthError } from './auth'

const CLAUDE_AUTHORIZE_URL = 'https://claude.ai/oauth/authorize'
const CLAUDE_CONSOLE_BASE = 'https://console.anthropic.com'
const CLAUDE_CODE_CLIENT_ID = '9d1c250a-e61b-44d9-88ed-5944d1962f5e'
const CLAUDE_LOGIN_SCOPE = 'org:create_api_key user:profile user:inference'

/** Pool-entry oauth.json shape (demi-owned; refreshable when refreshToken present). */
export interface ClaudeCodeOAuthSecret {
  accessToken: string
  refreshToken?: string | null
  /** ISO-8601 access token expiry. */
  expiresAt?: string | null
  scopes?: string[] | null
  subscriptionType?: string | null
  rateLimitTier?: string | null
  emailAddress?: string | null
  [key: string]: unknown
}

export interface ClaudeCodeLoginOptions {
  signal?: AbortSignal
  /** Fires once with the authorize URL; the flow then waits on promptForCode. */
  onPending?: (pending: { verificationUrl: string; requiresCodeInput: true }) => void
  /** Collects the "code#state" string the vendor page shows after approval. */
  promptForCode: () => Promise<string>
  fetch?: typeof fetch
  consoleBase?: string
}

function tokenEndpoint(consoleBase: string): string {
  return `${consoleBase.replace(/\/+$/, '')}/v1/oauth/token`
}

async function requestTokens(
  fetchImpl: typeof fetch,
  consoleBase: string,
  body: Record<string, string>,
  signal?: AbortSignal,
): Promise<ClaudeCodeOAuthSecret> {
  const response = await fetchImpl(tokenEndpoint(consoleBase), {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
    signal,
  })
  if (!response.ok) {
    throw new ClaudeCodeAuthError('auth_invalid', `Claude OAuth token request failed with HTTP ${response.status}`)
  }
  const parsed: unknown = await response.json().catch(() => null)
  if (!isRecord(parsed)) throw new ClaudeCodeAuthError('auth_invalid', 'Claude OAuth token response is not a JSON object')
  const accessToken = nonEmptyString(parsed.access_token)
  if (!accessToken) throw new ClaudeCodeAuthError('auth_invalid', 'Claude OAuth token response is missing access_token')
  const expiresIn = Number(parsed.expires_in)
  const account = isRecord(parsed.account) ? parsed.account : {}
  return {
    accessToken,
    refreshToken: nonEmptyString(parsed.refresh_token) ?? null,
    expiresAt: Number.isFinite(expiresIn) && expiresIn > 0 ? new Date(Date.now() + expiresIn * 1000).toISOString() : null,
    scopes: typeof parsed.scope === 'string' ? parsed.scope.split(' ').filter(Boolean) : null,
    subscriptionType: nonEmptyString(parsed.subscription_type) ?? nonEmptyString(account.subscription_type) ?? null,
    ...(nonEmptyString(account.email_address) ? { emailAddress: nonEmptyString(account.email_address) } : {}),
  }
}

/** Runs the copy-back OAuth flow and returns a refreshable pool secret. */
export async function runClaudeCodeLogin(options: ClaudeCodeLoginOptions): Promise<ClaudeCodeOAuthSecret> {
  const fetchImpl = options.fetch ?? fetch
  const consoleBase = options.consoleBase ?? CLAUDE_CONSOLE_BASE

  const verifier = randomBytes(32).toString('base64url')
  const challenge = createHash('sha256').update(verifier).digest('base64url')
  const state = randomBytes(32).toString('base64url')
  const redirectUri = `${consoleBase.replace(/\/+$/, '')}/oauth/code/callback`

  const params = new URLSearchParams({
    code: 'true',
    client_id: CLAUDE_CODE_CLIENT_ID,
    response_type: 'code',
    redirect_uri: redirectUri,
    scope: CLAUDE_LOGIN_SCOPE,
    code_challenge: challenge,
    code_challenge_method: 'S256',
    state,
  })
  options.onPending?.({ verificationUrl: `${CLAUDE_AUTHORIZE_URL}?${params.toString()}`, requiresCodeInput: true })

  const pasted = (await options.promptForCode()).trim()
  if (!pasted) throw new ClaudeCodeAuthError('auth_invalid', 'Empty authorization code')
  const [code, returnedState] = pasted.split('#')
  if (!nonEmptyString(code)) throw new ClaudeCodeAuthError('auth_invalid', 'Authorization code is missing the code part')
  if (returnedState && returnedState !== state) {
    throw new ClaudeCodeAuthError('auth_invalid', 'Authorization code state mismatch — copy the full string from the callback page')
  }

  return requestTokens(fetchImpl, consoleBase, {
    grant_type: 'authorization_code',
    code: code!,
    state: returnedState ?? state,
    client_id: CLAUDE_CODE_CLIENT_ID,
    redirect_uri: redirectUri,
    code_verifier: verifier,
  }, options.signal)
}

/** Refreshes a pool secret in place; returns the renewed secret. */
export async function refreshClaudeCodeSecret(
  secret: ClaudeCodeOAuthSecret,
  options: { fetch?: typeof fetch; consoleBase?: string; signal?: AbortSignal } = {},
): Promise<ClaudeCodeOAuthSecret> {
  const refreshToken = nonEmptyString(secret.refreshToken)
  if (!refreshToken) throw new ClaudeCodeAuthError('auth_missing', 'Claude OAuth secret has no refreshToken to renew with')
  const renewed = await requestTokens(options.fetch ?? fetch, options.consoleBase ?? CLAUDE_CONSOLE_BASE, {
    grant_type: 'refresh_token',
    refresh_token: refreshToken,
    client_id: CLAUDE_CODE_CLIENT_ID,
  }, options.signal)
  // Field-wise merge: refresh responses may omit metadata the original login carried.
  return {
    ...secret,
    accessToken: renewed.accessToken,
    refreshToken: renewed.refreshToken ?? refreshToken,
    expiresAt: renewed.expiresAt ?? secret.expiresAt ?? null,
    scopes: renewed.scopes ?? secret.scopes ?? null,
    subscriptionType: renewed.subscriptionType ?? secret.subscriptionType ?? null,
    ...(nonEmptyString(renewed.emailAddress) ? { emailAddress: renewed.emailAddress } : {}),
  }
}

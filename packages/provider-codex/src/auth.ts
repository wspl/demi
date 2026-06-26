import { errorMessage, isRecord } from '@demi/utils'
import { Buffer } from 'node:buffer'
import { chmod, mkdir, open, readFile, rename, rm, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import type { ProviderAuthState } from '@demi/provider'

export type CodexAuthMode =
  | 'apiKey'
  | 'chatgpt'
  | 'chatgptAuthTokens'
  | 'agentIdentity'
  | 'personalAccessToken'
  | 'bedrockApiKey'

export interface CodexTokenData {
  id_token?: string | Record<string, unknown>
  access_token?: string
  refresh_token?: string
  account_id?: string | null
}

export interface CodexAuthDotJson {
  auth_mode?: CodexAuthMode
  OPENAI_API_KEY?: string | null
  tokens?: CodexTokenData | null
  last_refresh?: string | null
  agent_identity?: unknown
  personal_access_token?: string | null
  bedrock_api_key?: unknown
  [key: string]: unknown
}

export type CodexResolvedAuth =
  | {
      kind: 'chatgpt'
      mode: 'chatgpt' | 'chatgptAuthTokens'
      accessToken: string
      refreshToken: string | null
      accountId: string
      email: string | null
      isFedrampAccount: boolean
      expiresAt: Date | null
      authFile: string
    }
  | {
      kind: 'apiKey'
      mode: 'apiKey'
      apiKey: string
      authFile: string | null
    }
  | {
      kind: 'personalAccessToken'
      mode: 'personalAccessToken'
      accessToken: string
      accountId: string | null
      authFile: string
    }
  | {
      kind: 'agentIdentity'
      mode: 'agentIdentity'
      authorization: string
      accountId: string
      isFedrampAccount: boolean
      authFile: string
    }

export interface CodexAuthStore {
  status(): Promise<ProviderAuthState>
  resolveAuth(options?: { forceRefresh?: boolean }): Promise<CodexResolvedAuth>
}

export async function codexAuthStatus(options: FileCodexAuthStoreOptions = {}): Promise<ProviderAuthState> {
  return new FileCodexAuthStore(options).status()
}

export interface FileCodexAuthStoreOptions {
  codexHome?: string
  refresh?: CodexTokenRefresh
  now?: () => Date
  lockRetryDelayMs?: number
  lockTimeoutMs?: number
}

export interface RefreshTokenResponse {
  id_token?: string
  access_token?: string
  refresh_token?: string
}

export type CodexTokenRefresh = (refreshToken: string, signal?: AbortSignal) => Promise<RefreshTokenResponse>

const TOKEN_REFRESH_CLIENT_ID = 'app_EMoamEEZ73f0CkXaXp7hrann'
const TOKEN_REFRESH_URL = 'https://auth.openai.com/oauth/token'
const REFRESH_EXPIRY_SKEW_MS = 5 * 60 * 1000
const REFRESH_STALENESS_MS = 8 * 24 * 60 * 60 * 1000

export class FileCodexAuthStore implements CodexAuthStore {
  readonly codexHome: string
  readonly authFile: string

  private readonly refreshImpl: CodexTokenRefresh
  private readonly now: () => Date
  private readonly lockRetryDelayMs: number
  private readonly lockTimeoutMs: number

  constructor(options: FileCodexAuthStoreOptions = {}) {
    this.codexHome = options.codexHome ?? defaultCodexHome()
    this.authFile = join(this.codexHome, 'auth.json')
    this.refreshImpl = options.refresh ?? refreshCodexToken
    this.now = options.now ?? (() => new Date())
    this.lockRetryDelayMs = options.lockRetryDelayMs ?? 25
    this.lockTimeoutMs = options.lockTimeoutMs ?? 5_000
  }

  async status(): Promise<ProviderAuthState> {
    try {
      const auth = await this.resolveAuth()
      if (auth.kind === 'chatgpt') {
        return { status: 'authenticated', accountLabel: auth.email ?? auth.accountId }
      }
      if (auth.kind === 'apiKey') return { status: 'authenticated', accountLabel: 'OPENAI_API_KEY' }
      if (auth.kind === 'personalAccessToken') return { status: 'authenticated', accountLabel: auth.accountId ?? 'personal access token' }
      return { status: 'authenticated', accountLabel: auth.accountId }
    } catch (error) {
      if (error instanceof CodexAuthError && error.code === 'auth_missing') return { status: 'unauthenticated', message: error.message }
      if (error instanceof CodexAuthError && error.code === 'auth_unsupported') return { status: 'error', message: error.message }
      return { status: 'error', message: redactSecretText(error instanceof Error ? error.message : String(error)) }
    }
  }

  async resolveAuth(options: { forceRefresh?: boolean } = {}): Promise<CodexResolvedAuth> {
    const auth = await this.readAuthFile()
    const mode = resolvedAuthMode(auth)

    if (mode === 'bedrockApiKey') {
      throw new CodexAuthError('auth_unsupported', 'Codex provider does not support Bedrock auth')
    }
    if (mode === 'apiKey') {
      const key = nonEmptyString(auth.OPENAI_API_KEY)
      if (!key) throw new CodexAuthError('auth_missing', `No OPENAI_API_KEY found in ${this.authFile}`)
      return { kind: 'apiKey', mode, apiKey: key, authFile: this.authFile }
    }
    if (mode === 'personalAccessToken') {
      const token = nonEmptyString(auth.personal_access_token)
      if (!token) throw new CodexAuthError('auth_missing', `No personal access token found in ${this.authFile}`)
      const claims = parseChatGptClaims(token)
      return {
        kind: 'personalAccessToken',
        mode,
        accessToken: token,
        accountId: claims.accountId,
        authFile: this.authFile,
      }
    }
    if (mode === 'agentIdentity') {
      return resolveAgentIdentity(auth, this.authFile)
    }

    const tokens = auth.tokens
    if (!tokens || typeof tokens !== 'object') {
      throw new CodexAuthError('auth_missing', `No ChatGPT tokens found in ${this.authFile}`)
    }
    const accessToken = nonEmptyString(tokens.access_token)
    if (!accessToken) throw new CodexAuthError('auth_missing', `No ChatGPT access token found in ${this.authFile}`)

    const refreshToken = nonEmptyString(tokens.refresh_token)
    const claims = parseChatGptClaims(accessToken)
    const idClaims = parseIdTokenClaims(tokens.id_token)
    const accountId = nonEmptyString(tokens.account_id) ?? claims.accountId ?? idClaims.accountId
    if (!accountId) throw new CodexAuthError('auth_missing', `No ChatGPT account id found in ${this.authFile}`)

    const expiresAt = parseJwtExpiration(accessToken)
    const lastRefresh = parseDate(auth.last_refresh)
    const shouldRefresh =
      options.forceRefresh === true ||
      expiresWithin(expiresAt, this.now(), REFRESH_EXPIRY_SKEW_MS) ||
      olderThan(lastRefresh, this.now(), REFRESH_STALENESS_MS)

    if (shouldRefresh && refreshToken) {
      return this.refreshAndResolve(auth, refreshToken)
    }

    return {
      kind: 'chatgpt',
      mode,
      accessToken,
      refreshToken,
      accountId,
      email: idClaims.email ?? claims.email,
      isFedrampAccount: idClaims.isFedrampAccount || claims.isFedrampAccount,
      expiresAt,
      authFile: this.authFile,
    }
  }

  private async refreshAndResolve(auth: CodexAuthDotJson, refreshToken: string): Promise<CodexResolvedAuth> {
    return this.withAuthFileLock(async () => {
      const latest = await this.readAuthFile()
      const latestTokens = latest.tokens
      const latestRefreshToken =
        latestTokens && typeof latestTokens === 'object' ? nonEmptyString(latestTokens.refresh_token) ?? refreshToken : refreshToken
      const response = await this.refreshImpl(latestRefreshToken)
      const nextTokens: CodexTokenData = {
        ...(latestTokens && typeof latestTokens === 'object' ? latestTokens : {}),
        ...(response.id_token ? { id_token: response.id_token } : {}),
        ...(response.access_token ? { access_token: response.access_token } : {}),
        ...(response.refresh_token ? { refresh_token: response.refresh_token } : {}),
      }
      const nextAuth: CodexAuthDotJson = {
        ...latest,
        auth_mode: latest.auth_mode ?? auth.auth_mode ?? 'chatgpt',
        tokens: nextTokens,
        last_refresh: this.now().toISOString(),
      }
      await writeAuthJsonAtomic(this.authFile, nextAuth)
      return resolveChatGptAuthFromFile(nextAuth, this.authFile)
    })
  }

  private async readAuthFile(): Promise<CodexAuthDotJson> {
    try {
      return JSON.parse(await readFile(this.authFile, 'utf8')) as CodexAuthDotJson
    } catch (error) {
      if (isNodeError(error) && error.code === 'ENOENT') {
        throw new CodexAuthError('auth_missing', `Codex auth file not found: ${this.authFile}`)
      }
      throw new CodexAuthError('auth_invalid', `Failed to read Codex auth file ${this.authFile}: ${redactSecretText(errorMessage(error))}`)
    }
  }

  private async withAuthFileLock<T>(fn: () => Promise<T>): Promise<T> {
    const lockFile = `${this.authFile}.lock`
    await mkdir(dirname(this.authFile), { recursive: true })
    const started = Date.now()
    let handle: Awaited<ReturnType<typeof open>> | null = null
    while (!handle) {
      try {
        handle = await open(lockFile, 'wx', 0o600)
      } catch (error) {
        if (!isNodeError(error) || error.code !== 'EEXIST' || Date.now() - started > this.lockTimeoutMs) {
          throw new CodexAuthError('auth_lock_failed', `Failed to lock Codex auth file: ${redactSecretText(errorMessage(error))}`)
        }
        await delay(this.lockRetryDelayMs)
      }
    }

    try {
      return await fn()
    } finally {
      await handle.close().catch(() => undefined)
      await rm(lockFile, { force: true }).catch(() => undefined)
    }
  }
}

export class StaticCodexAuthStore implements CodexAuthStore {
  constructor(private readonly auth: CodexResolvedAuth) {}

  async status(): Promise<ProviderAuthState> {
    return { status: 'authenticated', accountLabel: this.auth.kind }
  }

  async resolveAuth(): Promise<CodexResolvedAuth> {
    return this.auth
  }
}

export class CodexAuthError extends Error {
  constructor(
    readonly code:
      | 'auth_missing'
      | 'auth_invalid'
      | 'auth_unsupported'
      | 'auth_refresh_failed'
      | 'auth_lock_failed',
    message: string,
  ) {
    super(message)
    this.name = 'CodexAuthError'
  }
}

export function defaultCodexHome(): string {
  return process.env.CODEX_HOME && process.env.CODEX_HOME.trim() ? process.env.CODEX_HOME : join(homedir(), '.codex')
}

export function resolvedAuthMode(auth: CodexAuthDotJson): CodexAuthMode {
  if (auth.auth_mode) return auth.auth_mode
  if (auth.personal_access_token) return 'personalAccessToken'
  if (auth.bedrock_api_key) return 'bedrockApiKey'
  if (auth.OPENAI_API_KEY) return 'apiKey'
  if (auth.agent_identity) return 'agentIdentity'
  return 'chatgpt'
}

export function parseJwtExpiration(jwt: string): Date | null {
  const payload = decodeJwtPayload(jwt)
  const exp = payload?.exp
  return typeof exp === 'number' ? new Date(exp * 1000) : null
}

export function parseChatGptClaims(jwt: string): {
  accountId: string | null
  email: string | null
  isFedrampAccount: boolean
} {
  const payload = decodeJwtPayload(jwt)
  const auth = isRecord(payload?.['https://api.openai.com/auth']) ? payload['https://api.openai.com/auth'] : null
  const profile = isRecord(payload?.['https://api.openai.com/profile']) ? payload['https://api.openai.com/profile'] : null
  return {
    accountId: stringOrNull(auth?.chatgpt_account_id),
    email: stringOrNull(payload?.email) ?? stringOrNull(profile?.email),
    isFedrampAccount: auth?.chatgpt_account_is_fedramp === true,
  }
}

export function parseIdTokenClaims(idToken: unknown): {
  accountId: string | null
  email: string | null
  isFedrampAccount: boolean
} {
  if (typeof idToken === 'string') return parseChatGptClaims(idToken)
  if (!isRecord(idToken)) return { accountId: null, email: null, isFedrampAccount: false }
  return {
    accountId: stringOrNull(idToken.chatgpt_account_id),
    email: stringOrNull(idToken.email),
    isFedrampAccount: idToken.chatgpt_account_is_fedramp === true,
  }
}

export async function refreshCodexToken(refreshToken: string, signal?: AbortSignal): Promise<RefreshTokenResponse> {
  const response = await fetch(TOKEN_REFRESH_URL, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      client_id: process.env.CODEX_APP_SERVER_LOGIN_CLIENT_ID || TOKEN_REFRESH_CLIENT_ID,
      grant_type: 'refresh_token',
      refresh_token: refreshToken,
    }),
    signal,
  })
  if (!response.ok) {
    throw new CodexAuthError('auth_refresh_failed', `Codex token refresh failed with HTTP ${response.status}`)
  }
  return (await response.json()) as RefreshTokenResponse
}

export function redactSecretText(text: string): string {
  return text
    .replace(/Bearer\s+[A-Za-z0-9._~+/=-]+/g, 'Bearer [REDACTED]')
    .replace(/(access_token|refresh_token|id_token|OPENAI_API_KEY)["'=:\s]+[A-Za-z0-9._~+/=-]+/gi, '$1=[REDACTED]')
}

function resolveChatGptAuthFromFile(auth: CodexAuthDotJson, authFile: string): CodexResolvedAuth {
  const tokens = auth.tokens
  if (!tokens || typeof tokens !== 'object') throw new CodexAuthError('auth_missing', `No ChatGPT tokens found in ${authFile}`)
  const accessToken = nonEmptyString(tokens.access_token)
  if (!accessToken) throw new CodexAuthError('auth_missing', `No ChatGPT access token found in ${authFile}`)
  const claims = parseChatGptClaims(accessToken)
  const idClaims = parseIdTokenClaims(tokens.id_token)
  const accountId = nonEmptyString(tokens.account_id) ?? claims.accountId ?? idClaims.accountId
  if (!accountId) throw new CodexAuthError('auth_missing', `No ChatGPT account id found in ${authFile}`)
  return {
    kind: 'chatgpt',
    mode: resolvedAuthMode(auth) === 'chatgptAuthTokens' ? 'chatgptAuthTokens' : 'chatgpt',
    accessToken,
    refreshToken: nonEmptyString(tokens.refresh_token),
    accountId,
    email: idClaims.email ?? claims.email,
    isFedrampAccount: idClaims.isFedrampAccount || claims.isFedrampAccount,
    expiresAt: parseJwtExpiration(accessToken),
    authFile,
  }
}

function resolveAgentIdentity(auth: CodexAuthDotJson, authFile: string): CodexResolvedAuth {
  const value = auth.agent_identity
  if (!isRecord(value)) {
    throw new CodexAuthError('auth_unsupported', 'Codex agent identity auth requires an auth record')
  }
  const authorization = nonEmptyString(value.authorization)
  const accountId = nonEmptyString(value.account_id)
  if (!authorization || !accountId) {
    throw new CodexAuthError(
      'auth_unsupported',
      'Codex agent identity auth requires an authorization header generated by official Codex',
    )
  }
  return {
    kind: 'agentIdentity',
    mode: 'agentIdentity',
    authorization,
    accountId,
    isFedrampAccount: value.chatgpt_account_is_fedramp === true,
    authFile,
  }
}

async function writeAuthJsonAtomic(authFile: string, auth: CodexAuthDotJson): Promise<void> {
  await mkdir(dirname(authFile), { recursive: true })
  const temp = `${authFile}.${process.pid}.${Date.now()}.tmp`
  await writeFile(temp, `${JSON.stringify(auth, null, 2)}\n`, { mode: 0o600 })
  await chmod(temp, 0o600)
  await rename(temp, authFile)
}

function decodeJwtPayload(jwt: string): Record<string, unknown> | null {
  const parts = jwt.split('.')
  if (parts.length !== 3 || !parts[1]) return null
  try {
    const normalized = parts[1].replace(/-/g, '+').replace(/_/g, '/')
    const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, '=')
    return JSON.parse(Buffer.from(padded, 'base64').toString('utf8')) as Record<string, unknown>
  } catch {
    return null
  }
}

function expiresWithin(expiresAt: Date | null, now: Date, skewMs: number): boolean {
  return expiresAt !== null && expiresAt.getTime() - now.getTime() <= skewMs
}

function olderThan(value: Date | null, now: Date, ageMs: number): boolean {
  return value !== null && now.getTime() - value.getTime() >= ageMs
}

function parseDate(value: unknown): Date | null {
  if (typeof value !== 'string' || !value) return null
  const ms = Date.parse(value)
  return Number.isFinite(ms) ? new Date(ms) : null
}

function nonEmptyString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null
}

function stringOrNull(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

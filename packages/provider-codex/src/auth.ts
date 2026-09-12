import {
  delay,
  errorCode,
  errorMessage,
} from '@demicodes/utils'
import {
  chmod,
  mkdir,
  open,
  readFile,
  rename,
  rm,
  writeFile
} from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import {
  parseProviderData,
  parseProviderJson,
  parseProviderJwt,
  redactCredentialText,
  type ProviderAuthState
} from '@demicodes/provider'

import {
  codexAuthFileSchema, codexJwtClaimsSchema, codexRefreshResponseSchema,
  type CodexAuthDotJson, type CodexAuthMode, type CodexTokenData,
} from './auth-schemas'
export type { CodexAuthDotJson, CodexAuthMode, CodexTokenData, RefreshTokenResponse } from './auth-schemas'

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
      email: string | null
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

export async function codexAuthStatus(
  options: FileCodexAuthStoreOptions = {}
): Promise<ProviderAuthState> {
  return new FileCodexAuthStore(options).status()
}

export interface FileCodexAuthStoreOptions {
  codexHome?: string
  /** Override auth.json path (e.g. demi credential pool entry). */
  authFile?: string
  refresh?: CodexTokenRefresh
  now?: () => Date
  lockRetryDelayMs?: number
  lockTimeoutMs?: number
}

/** The store validates the raw refresh result, including injected implementations. */
export type CodexTokenRefresh = (
  refreshToken: string,
  signal?: AbortSignal
) => Promise<unknown>

/**
 * Codex error text may embed the env-provided API key by name; redact it
 * alongside the standard fields.
 */
const SECRET_FIELD_PATTERNS = ['OPENAI_API_KEY']

export function redactCodexSecretText(text: string): string {
  return redactCredentialText(text, SECRET_FIELD_PATTERNS)
}

const CODEX_OAUTH_CLIENT_ID = 'app_EMoamEEZ73f0CkXaXp7hrann'
const TOKEN_REFRESH_URL = 'https://auth.openai.com/oauth/token'

/** OAuth client id shared by token refresh and device-code login. */
export function codexOauthClientId(): string {
  return process.env.CODEX_APP_SERVER_LOGIN_CLIENT_ID || CODEX_OAUTH_CLIENT_ID
}
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
    this.authFile = options.authFile ?? join(this.codexHome, 'auth.json')
    this.refreshImpl = options.refresh ?? refreshCodexToken
    this.now = options.now ?? (() => new Date())
    this.lockRetryDelayMs = options.lockRetryDelayMs ?? 25
    this.lockTimeoutMs = options.lockTimeoutMs ?? 5_000
  }

  async status(): Promise<ProviderAuthState> {
    try {
      const auth = await this.resolveAuth()
      if (auth.kind === 'chatgpt') {
        return {
          status: 'authenticated',
          accountLabel: auth.email ?? auth.accountId
        }
      }
      if (auth.kind === 'apiKey')
        return {
          status: 'authenticated',
          accountLabel: 'OPENAI_API_KEY'
        }
      if (auth.kind === 'personalAccessToken')
        return {
          status: 'authenticated',
          accountLabel: auth.accountId ?? 'personal access token'
        }
      return { status: 'authenticated', accountLabel: auth.accountId }
    } catch (error) {
      if (error instanceof CodexAuthError && error.code === 'auth_missing')
        return {
        status: 'unauthenticated',
        message: error.message
      }
      if (error instanceof CodexAuthError && error.code === 'auth_unsupported')
        return {
        status: 'error',
        message: error.message
      }
      return {
        status: 'error',
        message: redactCodexSecretText(error instanceof Error
          ? error.message
          : String(error))
      }
    }
  }

  async resolveAuth(
    options: { forceRefresh?: boolean } = {}
  ): Promise<CodexResolvedAuth> {
    const auth = await this.readAuthFile()
    const resolved = resolveCodexAuth(auth, this.authFile)
    if (resolved.kind !== 'chatgpt') {
      return resolved
    }
    const lastRefresh = auth.last_refresh ? new Date(auth.last_refresh) : null
    const shouldRefresh = options.forceRefresh === true
      || expiresWithin(resolved.expiresAt, this.now(), REFRESH_EXPIRY_SKEW_MS)
      || olderThan(lastRefresh, this.now(), REFRESH_STALENESS_MS)
    if (shouldRefresh && resolved.refreshToken) {
      return this.refreshAndResolve()
    }
    return resolved
  }

  private async refreshAndResolve(): Promise<CodexResolvedAuth> {
    return this.withAuthFileLock(async () => {
      const latest = await this.readAuthFile()
      const resolved = resolveCodexAuth(latest, this.authFile)
      if (resolved.kind !== 'chatgpt' || !resolved.refreshToken) {
        return resolved
      }
      const response = parseProviderData(
        codexRefreshResponseSchema,
        await this.refreshImpl(resolved.refreshToken),
        'Codex token refresh',
      )
      const nextTokens: CodexTokenData = {
        ...latest.tokens,
        access_token: response.access_token,
        ...(response.id_token !== undefined ? { id_token: response.id_token } : {}),
        ...(response.refresh_token !== undefined ? { refresh_token: response.refresh_token } : {}),
      }
      const nextAuth: CodexAuthDotJson = {
        ...latest,
        auth_mode: resolved.mode,
        tokens: nextTokens,
        last_refresh: this.now().toISOString(),
      }
      // Resolve claims before committing so a bad refresh cannot replace working credentials.
      const nextResolved = resolveCodexAuth(nextAuth, this.authFile)
      await writeAuthJsonAtomic(this.authFile, nextAuth)
      return nextResolved
    })
  }

  private async readAuthFile(): Promise<CodexAuthDotJson> {
    try {
      return parseCodexAuthJson(await readFile(this.authFile, 'utf8'))
    } catch (error) {
      if (errorCode(error) === 'ENOENT') {
        throw new CodexAuthError(
          'auth_missing',
          `Codex auth file not found: ${this.authFile}`
        )
      }
      throw new CodexAuthError(
        'auth_invalid',
        `Failed to read Codex auth file ${this.authFile}: ${redactCodexSecretText(errorMessage(error))}`
      )
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
        if (errorCode(error) !== 'EEXIST'
          || Date.now() - started > this.lockTimeoutMs) {
          throw new CodexAuthError(
            'auth_lock_failed',
            `Failed to lock Codex auth file: ${redactCodexSecretText(errorMessage(error))}`
          )
        }
        await delay(this.lockRetryDelayMs)
      }
    }

    try {
      return await fn()
    } finally {
      try {
        await handle.close()
      } finally {
        await rm(lockFile, { force: true })
      }
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
      | 'auth_login_failed'
      | 'auth_lock_failed',
    message: string,
  ) {
    super(message)
    this.name = 'CodexAuthError'
  }
}

export function defaultCodexHome(): string {
  return process.env.CODEX_HOME
    && process.env.CODEX_HOME.trim() ? process.env.CODEX_HOME : join(
    homedir(),
    '.codex'
  )
}

export function resolvedAuthMode(auth: CodexAuthDotJson): CodexAuthMode {
  if (auth.auth_mode)
    return auth.auth_mode
  if (auth.personal_access_token)
    return 'personalAccessToken'
  if (auth.bedrock_api_key)
    return 'bedrockApiKey'
  if (auth.OPENAI_API_KEY)
    return 'apiKey'
  if (auth.agent_identity)
    return 'agentIdentity'
  return 'chatgpt'
}

export function parseCodexAuthJson(text: string): CodexAuthDotJson {
  try {
    return parseProviderJson(codexAuthFileSchema, text, 'Codex auth material')
  } catch (error) {
    throw new CodexAuthError('auth_invalid', errorMessage(error))
  }
}

export function parseJwtExpiration(jwt: string): Date | null {
  const exp = readJwtClaims(jwt)?.exp
  return exp === undefined ? null : new Date(exp * 1000)
}

export function parseChatGptClaims(jwt: string): {
  accountId: string | null
  email: string | null
  isFedrampAccount: boolean
} {
  const payload = readJwtClaims(jwt)
  const auth = payload?.['https://api.openai.com/auth']
  return {
    accountId: auth?.chatgpt_account_id ?? null,
    email: payload?.email ?? payload?.['https://api.openai.com/profile']?.email ?? null,
    isFedrampAccount: auth?.chatgpt_account_is_fedramp ?? false,
  }
}

export function parseIdTokenClaims(idToken: CodexTokenData['id_token']): {
  accountId: string | null
  email: string | null
  isFedrampAccount: boolean
} {
  if (typeof idToken === 'string') {
    return parseChatGptClaims(idToken)
  }
  return {
    accountId: idToken?.chatgpt_account_id ?? null,
    email: idToken?.email ?? null,
    isFedrampAccount: idToken?.chatgpt_account_is_fedramp ?? false,
  }
}

export async function refreshCodexToken(
  refreshToken: string,
  signal?: AbortSignal
): Promise<unknown> {
  const response = await fetch(TOKEN_REFRESH_URL, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      client_id: codexOauthClientId(),
      grant_type: 'refresh_token',
      refresh_token: refreshToken,
    }),
    signal,
  })
  if (!response.ok) {
    throw new CodexAuthError(
      'auth_refresh_failed',
      `Codex token refresh failed with HTTP ${response.status}`
    )
  }
  try {
    return await response.json()
  } catch {
    throw new CodexAuthError('auth_refresh_failed', 'Codex token refresh returned invalid JSON')
  }
}


/** Resolves validated auth material without refreshing, IO, or persistent side effects. */
export function resolveCodexAuth(auth: CodexAuthDotJson, authFile: string): CodexResolvedAuth {
  const mode = resolvedAuthMode(auth)
  switch (mode) {
    case 'bedrockApiKey':
      throw new CodexAuthError('auth_unsupported', 'Codex provider does not support Bedrock auth')
    case 'apiKey':
      if (!auth.OPENAI_API_KEY) {
        throw new CodexAuthError('auth_missing', `No OPENAI_API_KEY found in ${authFile}`)
      }
      return { kind: 'apiKey', mode, apiKey: auth.OPENAI_API_KEY, authFile }
    case 'personalAccessToken': {
      if (!auth.personal_access_token) {
        throw new CodexAuthError('auth_missing', `No personal access token found in ${authFile}`)
      }
      const claims = parseChatGptClaims(auth.personal_access_token)
      return {
        kind: 'personalAccessToken', mode, accessToken: auth.personal_access_token,
        accountId: claims.accountId, email: claims.email, authFile,
      }
    }
    case 'agentIdentity': {
      const identity = auth.agent_identity
      if (!identity) {
        throw new CodexAuthError('auth_missing', `No agent identity found in ${authFile}`)
      }
      return {
        kind: 'agentIdentity', mode, authorization: identity.authorization,
        accountId: identity.account_id,
        isFedrampAccount: identity.chatgpt_account_is_fedramp ?? false,
        authFile,
      }
    }
    case 'chatgpt':
    case 'chatgptAuthTokens': {
      const tokens = auth.tokens
      if (!tokens?.access_token) {
        throw new CodexAuthError('auth_missing', `No ChatGPT access token found in ${authFile}`)
      }
      const claims = readJwtClaims(tokens.access_token)
      const idClaims = parseIdTokenClaims(tokens.id_token)
      const authClaims = claims?.['https://api.openai.com/auth']
      const accountId = tokens.account_id ?? authClaims?.chatgpt_account_id ?? idClaims.accountId
      if (!accountId) {
        throw new CodexAuthError('auth_missing', `No ChatGPT account id found in ${authFile}`)
      }
      return {
        kind: 'chatgpt', mode, accessToken: tokens.access_token,
        refreshToken: tokens.refresh_token ?? null, accountId,
        email: idClaims.email ?? claims?.email ?? claims?.['https://api.openai.com/profile']?.email ?? null,
        isFedrampAccount: idClaims.isFedrampAccount || (authClaims?.chatgpt_account_is_fedramp ?? false),
        expiresAt: claims?.exp === undefined ? null : new Date(claims.exp * 1000),
        authFile,
      }
    }
  }
}

async function writeAuthJsonAtomic(
  authFile: string,
  auth: CodexAuthDotJson
): Promise<void> {
  await mkdir(dirname(authFile), { recursive: true })
  const temp = `${authFile}.${process.pid}.${Date.now()}.tmp`
  try {
    await writeFile(temp, `${JSON.stringify(auth, null, 2)}\n`, { mode: 0o600 })
    await chmod(temp, 0o600)
    await rename(temp, authFile)
  } finally {
    await rm(temp, { force: true })
  }
}

/** Decodes claims for metadata only; this does not verify the JWT signature. */
function readJwtClaims(jwt: string) {
  return parseProviderJwt(codexJwtClaimsSchema, jwt, 'Codex JWT claims')
}

function expiresWithin(
  expiresAt: Date | null,
  now: Date,
  skewMs: number
): boolean {
  return expiresAt !== null && expiresAt.getTime() - now.getTime() <= skewMs
}

function olderThan(value: Date | null, now: Date, ageMs: number): boolean {
  return value !== null && now.getTime() - value.getTime() >= ageMs
}

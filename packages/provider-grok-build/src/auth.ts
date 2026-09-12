import {
  delay,
  errorCode,
  errorMessage,
  nonEmptyString,
} from '@demicodes/utils'
import {
  open,
  readFile,
  rename,
  rm,
  writeFile,
  mkdir,
  chmod,
  stat
} from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import process from 'node:process'
import {
  parseProviderData,
  parseProviderJwt,
  redactCredentialText,
  type ProviderAuthState
} from '@demicodes/provider'

import {
  grokClaimsSchema,
  grokTokenResponseSchema,
  parseGrokAuthData,
  type GrokAuthDotJson,
  type GrokAuthEntry,
} from './auth-schemas'
export type { GrokAuthDotJson, GrokAuthEntry, GrokRefreshTokenResponse } from './auth-schemas'

export interface GrokResolvedAuth {
  accessToken: string
  refreshToken: string | null
  expiresAt: Date | null
  email: string | null
  userId: string | null
  principalType: string | null
  principalId: string | null
  issuer: string | null
  clientId: string | null
  entryKey: string
  authFile: string
}

export interface GrokAuthStore {
  status(): Promise<ProviderAuthState>
  resolveAuth(options?: { forceRefresh?: boolean }): Promise<GrokResolvedAuth>
}

export interface FileGrokAuthStoreOptions {
  grokHome?: string
  /** Override auth.json path (e.g. demi credential pool entry). */
  authFile?: string
  /**
   * Prefer this map key when the auth file has multiple OIDC entries.
   * When unset, uses {@link selectAuthEntry} scoring.
   */
  entryKey?: string
  refresh?: GrokTokenRefresh
  now?: () => Date
  lockRetryDelayMs?: number
  lockTimeoutMs?: number
}

export type GrokTokenRefresh = (
  input: {
    refreshToken: string
    clientId: string
    tokenEndpoint: string
    principalType?: string | null
    principalId?: string | null
  },
  signal?: AbortSignal,
) => Promise<unknown>

/**
 * Grok auth.json stores the access token under `key`; redact it alongside the
 * standard fields.
 */
const SECRET_FIELD_PATTERNS = ['\\bkey\\b']

function redactGrokSecretText(text: string): string {
  return redactCredentialText(text, SECRET_FIELD_PATTERNS)
}

const DEFAULT_TOKEN_ENDPOINT = 'https://auth.x.ai/oauth2/token'
const REFRESH_EXPIRY_SKEW_MS = 5 * 60 * 1000

export async function grokBuildAuthStatus(
  options: FileGrokAuthStoreOptions = {}
): Promise<ProviderAuthState> {
  return new FileGrokAuthStore(options).status()
}

export class FileGrokAuthStore implements GrokAuthStore {
  readonly grokHome: string
  readonly authFile: string
  readonly entryKey: string | null

  private readonly refreshImpl: GrokTokenRefresh
  private readonly now: () => Date
  private readonly lockRetryDelayMs: number
  private readonly lockTimeoutMs: number

  constructor(options: FileGrokAuthStoreOptions = {}) {
    this.grokHome = options.grokHome ?? defaultGrokHome()
    this.authFile = options.authFile ?? join(this.grokHome, 'auth.json')
    this.entryKey = nonEmptyString(options.entryKey) ?? null
    this.refreshImpl = options.refresh ?? refreshGrokOidcToken
    this.now = options.now ?? (() => new Date())
    this.lockRetryDelayMs = options.lockRetryDelayMs ?? 25
    // Must cover the lock holder's full token refresh (a network round-trip),
    // not just a file write — contenders wait for the result instead of failing.
    this.lockTimeoutMs = options.lockTimeoutMs ?? 30_000
  }

  async status(): Promise<ProviderAuthState> {
    try {
      const auth = await this.resolveAuth()
      return {
        status: 'authenticated',
        accountLabel: auth.email ?? auth.entryKey
      }
    } catch (error) {
      if (error instanceof GrokAuthError && error.code === 'auth_missing') {
        return { status: 'unauthenticated', message: error.message }
      }
      return {
        status: 'error',
        message: redactGrokSecretText(error instanceof Error
          ? error.message
          : String(error))
      }
    }
  }

  async resolveAuth(
    options: { forceRefresh?: boolean } = {}
  ): Promise<GrokResolvedAuth> {
    const file = await this.readAuthFile()
    const selected = this.entryKey
      ? selectAuthEntryByKey(file, this.entryKey)
      : selectAuthEntry(file)
    if (!selected) {
      throw new GrokAuthError(
        this.entryKey ? 'auth_invalid' : 'auth_missing',
        this.entryKey
          ? `No Grok OAuth entry "${this.entryKey}" in ${this.authFile}`
          : `No Grok OAuth session found in ${this.authFile}. Run \`grok login\` first.`,
      )
    }

    const { entryKey, entry } = selected
    const accessToken = entry.key

    const refreshToken = entry.refresh_token ?? null
    const clientId = entry.oidc_client_id
      ?? parseClientIdFromEntryKey(entryKey)
    const issuer = entry.oidc_issuer
      ?? parseIssuerFromEntryKey(entryKey)
    const expiresAt = parseExpiresAt(entry.expires_at)
      ?? parseJwtExpiration(accessToken)
    const shouldRefresh =
      options.forceRefresh === true || expiresWithin(
        expiresAt,
        this.now(),
        REFRESH_EXPIRY_SKEW_MS
      )

    if (shouldRefresh && refreshToken && clientId) {
      return this.refreshAndResolve(accessToken, entryKey)
    }

    return resolvedAuthFromEntry(entry, accessToken, {
      refreshToken,
      expiresAt,
      issuer,
      clientId,
      entryKey,
      authFile: this.authFile,
    })
  }

  private async refreshAndResolve(
    staleAccessToken: string,
    entryKey: string,
  ): Promise<GrokResolvedAuth> {
    return this.withAuthFileLock(async () => {
      const latest = await this.readAuthFile()
      const latestEntry = latest[entryKey]
      if (!latestEntry) {
        throw new GrokAuthError(
          'auth_missing',
          `Grok auth entry "${entryKey}" disappeared during refresh`
        )
      }
      const refreshToken = latestEntry.refresh_token
      const clientId = latestEntry.oidc_client_id
        ?? parseClientIdFromEntryKey(entryKey)
      const issuer = latestEntry.oidc_issuer
        ?? parseIssuerFromEntryKey(entryKey)

      // Another process may have refreshed while we waited for the lock. If the
      // token changed and is no longer near expiry, use it — refreshing again
      // wastes a round-trip and needlessly rotates the refresh token.
      const latestAccessToken = latestEntry.key
      if (latestAccessToken !== staleAccessToken) {
        const latestExpiresAt = parseExpiresAt(latestEntry.expires_at)
          ?? parseJwtExpiration(latestAccessToken)
        if (!expiresWithin(
          latestExpiresAt,
          this.now(),
          REFRESH_EXPIRY_SKEW_MS
        )) {
          return resolvedAuthFromEntry(latestEntry, latestAccessToken, {
            refreshToken: latestEntry.refresh_token ?? null,
            expiresAt: latestExpiresAt,
            issuer,
            clientId,
            entryKey,
            authFile: this.authFile,
          })
        }
      }

      if (!refreshToken || !clientId) {
        throw new GrokAuthError('auth_refresh_failed', 'Grok refresh credentials are missing')
      }
      const response = parseProviderData(grokTokenResponseSchema, await this.refreshImpl({
        refreshToken,
        clientId,
        tokenEndpoint: tokenEndpointForIssuer(issuer),
        principalType: latestEntry.principal_type,
        principalId: latestEntry.principal_id,
      }), 'Grok token refresh')
      const accessToken = response.access_token
      const tokenExpiry = parseJwtExpiration(accessToken)
      const expiresAt = response.expires_in === undefined
        ? tokenExpiry
        : new Date(this.now().getTime() + response.expires_in * 1000)

      const nextEntry: GrokAuthEntry = {
        ...latestEntry,
        key: accessToken,
        ...(response.refresh_token !== undefined ? {
          refresh_token: response.refresh_token
        } : {}),
        ...(expiresAt ? { expires_at: expiresAt.toISOString() } : {}),
      }
      if (expiresAt === null) {
        delete nextEntry.expires_at
      }
      const nextFile = parseGrokAuthData({ ...latest, [entryKey]: nextEntry })
      await writeAuthJsonAtomic(this.authFile, nextFile)

      return resolvedAuthFromEntry(nextEntry, accessToken, {
        refreshToken: nextEntry.refresh_token ?? null,
        expiresAt,
        issuer,
        clientId,
        entryKey,
        authFile: this.authFile,
      })
    })
  }

  private async readAuthFile(): Promise<GrokAuthDotJson> {
    try {
      return parseGrokAuthJson(await readFile(this.authFile, 'utf8'))
    } catch (error) {
      if (error instanceof GrokAuthError)
        throw error
      if (errorCode(error) === 'ENOENT') {
        throw new GrokAuthError(
          'auth_missing',
          `Grok auth file not found: ${this.authFile}. Run \`grok login\` first.`
        )
      }
      throw new GrokAuthError(
        'auth_invalid',
        `Failed to read Grok auth file ${this.authFile}: ${redactGrokSecretText(errorMessage(error))}`,
      )
    }
  }

  private async withAuthFileLock<T>(fn: () => Promise<T>): Promise<T> {
    const lockFile = `${this.authFile}.lock`
    await mkdir(dirname(this.authFile), { recursive: true })
    const started = Date.now()
    let handle: Awaited<ReturnType<typeof open>> | null = null
    let brokeStaleLock = false
    while (!handle) {
      try {
        handle = await open(lockFile, 'wx', 0o600)
      } catch (error) {
        if (errorCode(error) !== 'EEXIST') {
          throw new GrokAuthError(
            'auth_lock_failed',
            `Failed to lock Grok auth file: ${redactGrokSecretText(errorMessage(error))}`
          )
        }
        // Grok CLI writes `auth.json.lock` as `pid:unix_ts` and may leave it behind
        // after a crash. Steal only abandoned locks; wait if another live process holds it.
        const staleIdentity = brokeStaleLock
          ? null
          : await fileIdentity(lockFile)
        if (staleIdentity && (await isAbandonedGrokAuthLock(
          lockFile,
          this.now()
        ))) {
          if (await removeLockFileIfSame(lockFile, staleIdentity)) {
            brokeStaleLock = true
          }
          continue
        }
        if (Date.now() - started > this.lockTimeoutMs) {
          throw new GrokAuthError(
            'auth_lock_failed',
            `Timed out waiting for Grok auth lock ${lockFile}. If no other Grok process is running, delete the lock file and retry.`,
          )
        }
        await delay(this.lockRetryDelayMs)
      }
    }

    try {
      // Match Grok CLI lock payload shape so concurrent tools can detect ownership.
      await writeFile(
        lockFile,
        `${process.pid}:${Math.floor(this.now().getTime() / 1000)}`,
        { mode: 0o600 }
      )
      return await fn()
    } finally {
      try {
        const ownedIdentity = toFileIdentity(await handle.stat())
        await removeLockFileIfSame(lockFile, ownedIdentity)
      } finally {
        await handle.close()
      }
    }
  }
}

export class StaticGrokAuthStore implements GrokAuthStore {
  constructor(private readonly auth: GrokResolvedAuth) {}

  async status(): Promise<ProviderAuthState> {
    return {
      status: 'authenticated',
      accountLabel: this.auth.email ?? this.auth.entryKey
    }
  }

  async resolveAuth(): Promise<GrokResolvedAuth> {
    return this.auth
  }
}

export class GrokAuthError extends Error {
  constructor(
    readonly code: 'auth_missing'
      | 'auth_invalid'
      | 'auth_refresh_failed'
      | 'auth_lock_failed',
    message: string,
  ) {
    super(message)
    this.name = 'GrokAuthError'
  }
}

export function defaultGrokHome(): string {
  const fromEnv = process.env.GROK_HOME
  return fromEnv && fromEnv.trim() ? fromEnv : join(homedir(), '.grok')
}

export function selectAuthEntry(
  file: GrokAuthDotJson
): {
  entryKey: string;
  entry: GrokAuthEntry
} | null {
  const candidates: Array<{
    entryKey: string;
    entry: GrokAuthEntry;
    score: number
  }> = []
  for (const [entryKey, entry] of Object.entries(file)) {
    let score = 0
    if (entry.auth_mode === 'oidc')
      score += 4
    if (entry.refresh_token)
      score += 2
    if (entryKey.includes('auth.x.ai')
      || entry.oidc_issuer === 'https://auth.x.ai') {
      score += 1
    }
    candidates.push({ entryKey, entry, score })
  }
  candidates.sort((a, b) => b.score - a.score
    || a.entryKey.localeCompare(b.entryKey))
  return candidates[0] ?? null
}

export function selectAuthEntryByKey(
  file: GrokAuthDotJson,
  entryKey: string,
): {
  entryKey: string;
  entry: GrokAuthEntry
} | null {
  const entry = file[entryKey]
  return entry ? { entryKey, entry } : null
}

export async function refreshGrokOidcToken(
  input: {
    refreshToken: string
    clientId: string
    tokenEndpoint: string
    principalType?: string | null
    principalId?: string | null
  },
  signal?: AbortSignal,
): Promise<unknown> {
  const body = new URLSearchParams({
    grant_type: 'refresh_token',
    refresh_token: input.refreshToken,
    client_id: input.clientId,
  })
  const principalType = nonEmptyString(input.principalType)
  if (principalType)
    body.set('principal_type', principalType)
  const principalId = nonEmptyString(input.principalId)
  if (principalId)
    body.set('principal_id', principalId)
  const response = await fetch(input.tokenEndpoint, {
    method: 'POST',
    headers: {
      'content-type': 'application/x-www-form-urlencoded',
      accept: 'application/json'
    },
    body,
    signal,
  })
  if (!response.ok) {
    throw new GrokAuthError(
      'auth_refresh_failed',
      `Grok token refresh failed with HTTP ${response.status}`
    )
  }
  try {
    return await response.json()
  } catch {
    throw new GrokAuthError('auth_refresh_failed', 'Grok token refresh returned invalid JSON')
  }
}

export function peekGrokAccessTokenPrincipal(
  accessToken: string
): {
  principalType: string
  principalId: string
  teamId: string | null
} | null {
  const payload = decodeGrokJwtPayload(accessToken)
  if (!payload)
    return null
  const principalType = payload.principal_type ?? payload.principalType
  const principalId = payload.principal_id ?? payload.principalId
  if (!principalType || !principalId)
    return null
  return {
    principalType,
    principalId,
    teamId: payload.team_id ?? null
  }
}

export function parseJwtExpiration(jwt: string): Date | null {
  const payload = decodeGrokJwtPayload(jwt)
  const exp = payload?.exp
  return exp === undefined ? null : new Date(exp * 1000)
}

function tokenEndpointForIssuer(issuer: string | null): string {
  if (!issuer)
    return DEFAULT_TOKEN_ENDPOINT
  if (issuer === 'https://auth.x.ai' || issuer === 'https://auth.x.ai/')
    return DEFAULT_TOKEN_ENDPOINT
  return `${issuer.replace(/\/$/, '')}/oauth2/token`
}

function parseIssuerFromEntryKey(entryKey: string): string | null {
  const sep = entryKey.indexOf('::')
  if (sep <= 0)
    return null
  return entryKey.slice(0, sep)
}

function parseClientIdFromEntryKey(entryKey: string): string | null {
  const sep = entryKey.indexOf('::')
  if (sep < 0 || sep === entryKey.length - 2)
    return null
  return entryKey.slice(sep + 2) || null
}

function parseExpiresAt(value: string | undefined): Date | null {
  return value === undefined ? null : new Date(value)
}

function expiresWithin(
  expiresAt: Date | null,
  now: Date,
  skewMs: number
): boolean {
  return expiresAt !== null && expiresAt.getTime() - now.getTime() <= skewMs
}

async function writeAuthJsonAtomic(
  authFile: string,
  auth: GrokAuthDotJson
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

export function decodeGrokJwtPayload(jwt: string) {
  return parseProviderJwt(grokClaimsSchema, jwt, 'Grok JWT claims')
}

export function parseGrokAuthJson(text: string): GrokAuthDotJson {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    throw new GrokAuthError('auth_invalid', 'Grok auth material is not valid JSON')
  }
  return parseGrokAuthData(value)
}

function resolvedAuthFromEntry(
  entry: GrokAuthEntry,
  accessToken: string,
  rest: {
    refreshToken: string | null
    expiresAt: Date | null
    issuer: string | null
    clientId: string | null
    entryKey: string
    authFile: string
  },
): GrokResolvedAuth {
  const peeked = peekGrokAccessTokenPrincipal(accessToken)
  return {
    accessToken,
    refreshToken: rest.refreshToken,
    expiresAt: rest.expiresAt,
    email: entry.email ?? null,
    userId: entry.user_id ?? peeked?.principalId
      ?? decodeGrokJwtPayload(accessToken)?.sub ?? null,
    principalType: entry.principal_type ?? peeked?.principalType
      ?? null,
    principalId: entry.principal_id ?? peeked?.principalId
      ?? null,
    issuer: rest.issuer,
    clientId: rest.clientId,
    entryKey: rest.entryKey,
    authFile: rest.authFile,
  }
}


/**
 * Grok CLI lock format is `pid:unix_seconds`. A valid live PID always owns its
 * lock.
 */
export async function isAbandonedGrokAuthLock(
  lockFile: string,
  now: Date,
  maxAgeMs = 30_000
): Promise<boolean> {
  try {
    const raw = (await readFile(lockFile, 'utf8')).trim()
    const match = /^(\d+):(\d+)$/.exec(raw)
    if (match) {
      const pid = Number(match[1])
      const tsSec = Number(match[2])
      if (Number.isFinite(pid) && pid > 0)
        return !isProcessAlive(pid)
      if (Number.isFinite(tsSec) && now.getTime() - tsSec * 1000 > maxAgeMs)
        return true
      return false
    }
    // Unknown lock payload: fall back to mtime age (covers empty/corrupt leftovers).
    const info = await stat(lockFile)
    return now.getTime() - info.mtimeMs > maxAgeMs
  } catch (error) {
    if (errorCode(error) === 'ENOENT') {
      return true
    }
    throw error
  }
}

interface FileIdentity {
  dev: number | bigint
  ino: number | bigint
}

function toFileIdentity(info: {
  dev: number | bigint;
  ino: number | bigint
}): FileIdentity {
  return { dev: info.dev, ino: info.ino }
}

async function fileIdentity(path: string): Promise<FileIdentity | null> {
  try {
    return toFileIdentity(await stat(path))
  } catch (error) {
    if (errorCode(error) === 'ENOENT') {
      return null
    }
    throw error
  }
}

async function removeLockFileIfSame(
  lockFile: string,
  expected: FileIdentity
): Promise<boolean> {
  const current = await fileIdentity(lockFile)
  if (!current || current.dev !== expected.dev || current.ino !== expected.ino)
    return false
  try {
    await rm(lockFile)
    return true
  } catch (error) {
    if (errorCode(error) === 'ENOENT') {
      return false
    }
    throw error
  }
}

function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    if (errorCode(error) === 'EPERM')
      return true
    return false
  }
}

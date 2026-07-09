import { delay, errorMessage, isRecord, nonEmptyString, stringOrNull } from '@demicodes/utils'
import { Buffer } from 'node:buffer'
import { open, readFile, rename, rm, writeFile, mkdir, chmod, stat } from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import process from 'node:process'
import type { ProviderAuthState } from '@demicodes/provider'

/** One credential entry as written by the Grok CLI (`~/.grok/auth.json`). */
export interface GrokAuthEntry {
  key?: string
  auth_mode?: string
  refresh_token?: string
  expires_at?: string
  oidc_issuer?: string
  oidc_client_id?: string
  email?: string
  first_name?: string
  user_id?: string
  team_id?: string
  [key: string]: unknown
}

/** Full auth.json map: key is typically `issuer::client_id`. */
export type GrokAuthDotJson = Record<string, GrokAuthEntry>

export interface GrokResolvedAuth {
  accessToken: string
  refreshToken: string | null
  expiresAt: Date | null
  email: string | null
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
  refresh?: GrokTokenRefresh
  now?: () => Date
  lockRetryDelayMs?: number
  lockTimeoutMs?: number
}

export interface GrokRefreshTokenResponse {
  access_token?: string
  refresh_token?: string
  expires_in?: number
  token_type?: string
}

export type GrokTokenRefresh = (
  input: { refreshToken: string; clientId: string; tokenEndpoint: string },
  signal?: AbortSignal,
) => Promise<GrokRefreshTokenResponse>

const DEFAULT_TOKEN_ENDPOINT = 'https://auth.x.ai/oauth2/token'
const REFRESH_EXPIRY_SKEW_MS = 5 * 60 * 1000

export async function grokBuildAuthStatus(options: FileGrokAuthStoreOptions = {}): Promise<ProviderAuthState> {
  return new FileGrokAuthStore(options).status()
}

export class FileGrokAuthStore implements GrokAuthStore {
  readonly grokHome: string
  readonly authFile: string

  private readonly refreshImpl: GrokTokenRefresh
  private readonly now: () => Date
  private readonly lockRetryDelayMs: number
  private readonly lockTimeoutMs: number

  constructor(options: FileGrokAuthStoreOptions = {}) {
    this.grokHome = options.grokHome ?? defaultGrokHome()
    this.authFile = join(this.grokHome, 'auth.json')
    this.refreshImpl = options.refresh ?? refreshGrokOidcToken
    this.now = options.now ?? (() => new Date())
    this.lockRetryDelayMs = options.lockRetryDelayMs ?? 25
    this.lockTimeoutMs = options.lockTimeoutMs ?? 5_000
  }

  async status(): Promise<ProviderAuthState> {
    try {
      const auth = await this.resolveAuth()
      return { status: 'authenticated', accountLabel: auth.email ?? auth.entryKey }
    } catch (error) {
      if (error instanceof GrokAuthError && error.code === 'auth_missing') {
        return { status: 'unauthenticated', message: error.message }
      }
      return { status: 'error', message: redactSecretText(error instanceof Error ? error.message : String(error)) }
    }
  }

  async resolveAuth(options: { forceRefresh?: boolean } = {}): Promise<GrokResolvedAuth> {
    const file = await this.readAuthFile()
    const selected = selectAuthEntry(file)
    if (!selected) {
      throw new GrokAuthError(
        'auth_missing',
        `No Grok OAuth session found in ${this.authFile}. Run \`grok login\` first.`,
      )
    }

    const { entryKey, entry } = selected
    const accessToken = nonEmptyString(entry.key)
    if (!accessToken) {
      throw new GrokAuthError('auth_missing', `Grok auth entry "${entryKey}" has no access token (key)`)
    }

    const refreshToken = nonEmptyString(entry.refresh_token) ?? null
    const clientId = nonEmptyString(entry.oidc_client_id) ?? parseClientIdFromEntryKey(entryKey)
    const issuer = nonEmptyString(entry.oidc_issuer) ?? parseIssuerFromEntryKey(entryKey)
    const expiresAt = parseExpiresAt(entry.expires_at) ?? parseJwtExpiration(accessToken)
    const shouldRefresh =
      options.forceRefresh === true || expiresWithin(expiresAt, this.now(), REFRESH_EXPIRY_SKEW_MS)

    if (shouldRefresh && refreshToken && clientId) {
      return this.refreshAndResolve(file, entryKey, {
        refreshToken,
        clientId,
        tokenEndpoint: tokenEndpointForIssuer(issuer),
      })
    }

    return {
      accessToken,
      refreshToken,
      expiresAt,
      email: stringOrNull(entry.email),
      issuer,
      clientId,
      entryKey,
      authFile: this.authFile,
    }
  }

  private async refreshAndResolve(
    _file: GrokAuthDotJson,
    entryKey: string,
    refreshInput: { refreshToken: string; clientId: string; tokenEndpoint: string },
  ): Promise<GrokResolvedAuth> {
    return this.withAuthFileLock(async () => {
      const latest = await this.readAuthFile()
      const latestEntry = latest[entryKey]
      if (!latestEntry || typeof latestEntry !== 'object') {
        throw new GrokAuthError('auth_missing', `Grok auth entry "${entryKey}" disappeared during refresh`)
      }
      const refreshToken = nonEmptyString(latestEntry.refresh_token) ?? refreshInput.refreshToken
      const clientId = nonEmptyString(latestEntry.oidc_client_id) ?? refreshInput.clientId
      const issuer = nonEmptyString(latestEntry.oidc_issuer) ?? parseIssuerFromEntryKey(entryKey)
      const response = await this.refreshImpl({
        refreshToken,
        clientId,
        tokenEndpoint: tokenEndpointForIssuer(issuer) || refreshInput.tokenEndpoint,
      })
      const accessToken = nonEmptyString(response.access_token)
      if (!accessToken) {
        throw new GrokAuthError('auth_refresh_failed', 'Grok token refresh returned no access_token')
      }

      const expiresAt =
        typeof response.expires_in === 'number' && Number.isFinite(response.expires_in)
          ? new Date(this.now().getTime() + response.expires_in * 1000)
          : parseJwtExpiration(accessToken)

      const nextEntry: GrokAuthEntry = {
        ...latestEntry,
        key: accessToken,
        ...(nonEmptyString(response.refresh_token) ? { refresh_token: response.refresh_token } : {}),
        ...(expiresAt ? { expires_at: expiresAt.toISOString() } : {}),
      }
      const nextFile: GrokAuthDotJson = { ...latest, [entryKey]: nextEntry }
      await writeAuthJsonAtomic(this.authFile, nextFile)

      return {
        accessToken,
        refreshToken: nonEmptyString(nextEntry.refresh_token) ?? null,
        expiresAt,
        email: stringOrNull(nextEntry.email),
        issuer,
        clientId,
        entryKey,
        authFile: this.authFile,
      }
    })
  }

  private async readAuthFile(): Promise<GrokAuthDotJson> {
    try {
      const parsed = JSON.parse(await readFile(this.authFile, 'utf8')) as unknown
      if (!isRecord(parsed)) {
        throw new GrokAuthError('auth_invalid', `Grok auth file is not an object: ${this.authFile}`)
      }
      return parsed as GrokAuthDotJson
    } catch (error) {
      if (error instanceof GrokAuthError) throw error
      if (isNodeError(error) && error.code === 'ENOENT') {
        throw new GrokAuthError('auth_missing', `Grok auth file not found: ${this.authFile}. Run \`grok login\` first.`)
      }
      throw new GrokAuthError(
        'auth_invalid',
        `Failed to read Grok auth file ${this.authFile}: ${redactSecretText(errorMessage(error))}`,
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
        if (!isNodeError(error) || error.code !== 'EEXIST') {
          throw new GrokAuthError('auth_lock_failed', `Failed to lock Grok auth file: ${redactSecretText(errorMessage(error))}`)
        }
        // Grok CLI writes `auth.json.lock` as `pid:unix_ts` and may leave it behind
        // after a crash. Steal only abandoned locks; wait if another live process holds it.
        if (!brokeStaleLock && (await isAbandonedGrokAuthLock(lockFile, this.now()))) {
          await rm(lockFile, { force: true }).catch(() => undefined)
          brokeStaleLock = true
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
      await writeFile(lockFile, `${process.pid}:${Math.floor(this.now().getTime() / 1000)}`, { mode: 0o600 })
      return await fn()
    } finally {
      await handle.close().catch(() => undefined)
      await rm(lockFile, { force: true }).catch(() => undefined)
    }
  }
}

export class StaticGrokAuthStore implements GrokAuthStore {
  constructor(private readonly auth: GrokResolvedAuth) {}

  async status(): Promise<ProviderAuthState> {
    return { status: 'authenticated', accountLabel: this.auth.email ?? this.auth.entryKey }
  }

  async resolveAuth(): Promise<GrokResolvedAuth> {
    return this.auth
  }
}

export class GrokAuthError extends Error {
  constructor(
    readonly code: 'auth_missing' | 'auth_invalid' | 'auth_refresh_failed' | 'auth_lock_failed',
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

export function selectAuthEntry(file: GrokAuthDotJson): { entryKey: string; entry: GrokAuthEntry } | null {
  const candidates: Array<{ entryKey: string; entry: GrokAuthEntry; score: number }> = []
  for (const [entryKey, value] of Object.entries(file)) {
    if (!isRecord(value)) continue
    const entry = value as GrokAuthEntry
    if (!nonEmptyString(entry.key)) continue
    let score = 0
    if (entry.auth_mode === 'oidc') score += 4
    if (nonEmptyString(entry.refresh_token)) score += 2
    if (entryKey.includes('auth.x.ai') || entry.oidc_issuer === 'https://auth.x.ai') score += 1
    candidates.push({ entryKey, entry, score })
  }
  candidates.sort((a, b) => b.score - a.score || a.entryKey.localeCompare(b.entryKey))
  return candidates[0] ?? null
}

export async function refreshGrokOidcToken(
  input: { refreshToken: string; clientId: string; tokenEndpoint: string },
  signal?: AbortSignal,
): Promise<GrokRefreshTokenResponse> {
  const body = new URLSearchParams({
    grant_type: 'refresh_token',
    refresh_token: input.refreshToken,
    client_id: input.clientId,
  })
  const response = await fetch(input.tokenEndpoint, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded', accept: 'application/json' },
    body,
    signal,
  })
  if (!response.ok) {
    throw new GrokAuthError('auth_refresh_failed', `Grok token refresh failed with HTTP ${response.status}`)
  }
  return (await response.json()) as GrokRefreshTokenResponse
}

export function redactSecretText(text: string): string {
  return text
    .replace(/Bearer\s+[A-Za-z0-9._~+/=-]+/g, 'Bearer [REDACTED]')
    .replace(/(access_token|refresh_token|id_token|\bkey\b)["'=:\s]+[A-Za-z0-9._~+/=-]+/gi, '$1=[REDACTED]')
}

export function parseJwtExpiration(jwt: string): Date | null {
  const payload = decodeJwtPayload(jwt)
  const exp = payload?.exp
  return typeof exp === 'number' ? new Date(exp * 1000) : null
}

function tokenEndpointForIssuer(issuer: string | null): string {
  if (!issuer) return DEFAULT_TOKEN_ENDPOINT
  if (issuer === 'https://auth.x.ai' || issuer === 'https://auth.x.ai/') return DEFAULT_TOKEN_ENDPOINT
  return `${issuer.replace(/\/$/, '')}/oauth2/token`
}

function parseIssuerFromEntryKey(entryKey: string): string | null {
  const sep = entryKey.indexOf('::')
  if (sep <= 0) return null
  return entryKey.slice(0, sep)
}

function parseClientIdFromEntryKey(entryKey: string): string | null {
  const sep = entryKey.indexOf('::')
  if (sep < 0 || sep === entryKey.length - 2) return null
  return entryKey.slice(sep + 2) || null
}

function parseExpiresAt(value: unknown): Date | null {
  if (typeof value !== 'string' || !value) return null
  const ms = Date.parse(value)
  return Number.isFinite(ms) ? new Date(ms) : null
}

function expiresWithin(expiresAt: Date | null, now: Date, skewMs: number): boolean {
  return expiresAt !== null && expiresAt.getTime() - now.getTime() <= skewMs
}

async function writeAuthJsonAtomic(authFile: string, auth: GrokAuthDotJson): Promise<void> {
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

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error
}

/** Grok CLI lock format is `pid:unix_seconds`. Steal only when the owner is dead or very stale. */
export async function isAbandonedGrokAuthLock(lockFile: string, now: Date, maxAgeMs = 30_000): Promise<boolean> {
  try {
    const raw = (await readFile(lockFile, 'utf8')).trim()
    const match = /^(\d+):(\d+)$/.exec(raw)
    if (match) {
      const pid = Number(match[1])
      const tsSec = Number(match[2])
      if (Number.isFinite(pid) && pid > 0 && !isProcessAlive(pid)) return true
      if (Number.isFinite(tsSec) && now.getTime() - tsSec * 1000 > maxAgeMs) return true
      return false
    }
    // Unknown lock payload: fall back to mtime age (covers empty/corrupt leftovers).
    const info = await stat(lockFile)
    return now.getTime() - info.mtimeMs > maxAgeMs
  } catch {
    return true
  }
}

function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    if (isNodeError(error) && error.code === 'EPERM') return true
    return false
  }
}

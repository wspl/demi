import {
  decodeJwtPayload,
  delay,
  errorCode,
  errorMessage,
  nonEmptyString
} from '@demicodes/utils'
import { z } from 'zod'
import {
  open,
  readFile,
  rm,
  writeFile,
  mkdir,
  stat
} from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import process from 'node:process'
import {
  redactCredentialText,
  reportedStringSchema,
  type ProviderAuthState
} from '@demicodes/provider'
import { writeJsonFileAtomic } from '@demicodes/provider/credentials-pool'

/**
 * One credential entry as written by the Grok CLI (`~/.grok/auth.json`).
 *
 * Demi reads the fields below; every other field survives a decode, because
 * the store rewrites the whole file on refresh and must not drop what a newer
 * Grok CLI wrote. `key` (the access token) is optional here: an entry without
 * one is a login the CLI never completed, and is skipped rather than rejected.
 * A field Demi does read must have the right type — a number `refresh_token`
 * is corrupt credential material, not a value to repair.
 */
export const grokAuthEntrySchema = z.looseObject({
  key: z.string().optional(),
  auth_mode: z.string().optional(),
  refresh_token: z.string().optional(),
  expires_at: z.string().optional(),
  oidc_issuer: z.string().optional(),
  oidc_client_id: z.string().optional(),
  email: z.string().optional(),
  first_name: z.string().optional(),
  last_name: z.string().optional(),
  user_id: z.string().optional(),
  team_id: z.string().optional(),
  principal_type: z.string().optional(),
  principal_id: z.string().optional(),
  organization_id: z.string().optional(),
})

export type GrokAuthEntry = z.infer<typeof grokAuthEntrySchema>

/** Full auth.json map: key is typically `issuer::client_id`. */
export const grokAuthDotJsonSchema = z.record(z.string(), grokAuthEntrySchema)

export type GrokAuthDotJson = z.infer<typeof grokAuthDotJsonSchema>

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

/**
 * The OAuth token endpoint's answer to a refresh. A response without an access
 * token is a failed refresh however it is spelled, so the field is required.
 */
export const grokRefreshTokenResponseSchema = z.looseObject({
  access_token: z.string().min(1),
  refresh_token: z.string().min(1).optional(),
  expires_in: z.number().positive().optional(),
  token_type: z.string().optional(),
})

export type GrokRefreshTokenResponse =
  z.infer<typeof grokRefreshTokenResponseSchema>

export type GrokTokenRefresh = (
  input: {
    refreshToken: string
    clientId: string
    tokenEndpoint: string
    principalType?: string | null
    principalId?: string | null
  },
  signal?: AbortSignal,
) => Promise<GrokRefreshTokenResponse>

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
        'auth_missing',
        this.entryKey
          ? `No Grok OAuth entry "${this.entryKey}" in ${this.authFile}`
          : `No Grok OAuth session found in ${this.authFile}. Run \`grok login\` first.`,
      )
    }

    const { entryKey, entry, accessToken } = selected
    const refreshToken = nonEmptyString(entry.refresh_token) ?? null
    const clientId = nonEmptyString(entry.oidc_client_id)
      ?? parseClientIdFromEntryKey(entryKey)
    const issuer = nonEmptyString(entry.oidc_issuer)
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
      return this.refreshAndResolve(accessToken, entryKey, {
        refreshToken,
        clientId,
        tokenEndpoint: tokenEndpointForIssuer(issuer),
        principalType: entry.principal_type ?? null,
        principalId: entry.principal_id ?? null,
      })
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
    refreshInput: {
      refreshToken: string
      clientId: string
      tokenEndpoint: string
      principalType?: string | null
      principalId?: string | null
    },
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
      const refreshToken = nonEmptyString(latestEntry.refresh_token)
        ?? refreshInput.refreshToken
      const clientId = nonEmptyString(latestEntry.oidc_client_id)
        ?? refreshInput.clientId
      const issuer = nonEmptyString(latestEntry.oidc_issuer)
        ?? parseIssuerFromEntryKey(entryKey)

      // Another process may have refreshed while we waited for the lock. If the
      // token changed and is no longer near expiry, use it — refreshing again
      // wastes a round-trip and needlessly rotates the refresh token.
      const latestAccessToken = nonEmptyString(latestEntry.key)
      if (latestAccessToken && latestAccessToken !== staleAccessToken) {
        const latestExpiresAt = parseExpiresAt(latestEntry.expires_at)
          ?? parseJwtExpiration(latestAccessToken)
        if (!expiresWithin(
          latestExpiresAt,
          this.now(),
          REFRESH_EXPIRY_SKEW_MS
        )) {
          return resolvedAuthFromEntry(latestEntry, latestAccessToken, {
            refreshToken: nonEmptyString(latestEntry.refresh_token) ?? null,
            expiresAt: latestExpiresAt,
            issuer,
            clientId,
            entryKey,
            authFile: this.authFile,
          })
        }
      }

      const response = await this.refreshImpl({
        refreshToken,
        clientId,
        tokenEndpoint: tokenEndpointForIssuer(issuer)
          || refreshInput.tokenEndpoint,
        principalType: latestEntry.principal_type
          ?? refreshInput.principalType,
        principalId: latestEntry.principal_id ?? refreshInput.principalId,
      })
      const accessToken = response.access_token
      const expiresAt = response.expires_in === undefined
        ? parseJwtExpiration(accessToken)
        : new Date(this.now().getTime() + response.expires_in * 1000)

      const nextEntry: GrokAuthEntry = {
        ...latestEntry,
        key: accessToken,
        ...(response.refresh_token
          ? { refresh_token: response.refresh_token }
          : {}),
        ...(expiresAt ? { expires_at: expiresAt.toISOString() } : {}),
      }
      const nextFile: GrokAuthDotJson = { ...latest, [entryKey]: nextEntry }
      await writeJsonFileAtomic(this.authFile, nextFile)

      return resolvedAuthFromEntry(nextEntry, accessToken, {
        refreshToken: nonEmptyString(nextEntry.refresh_token) ?? null,
        expiresAt,
        issuer,
        clientId,
        entryKey,
        authFile: this.authFile,
      })
    })
  }

  private async readAuthFile(): Promise<GrokAuthDotJson> {
    let text: string
    try {
      text = await readFile(this.authFile, 'utf8')
    } catch (error) {
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
    return parseGrokAuthDotJson(text, `Grok auth file ${this.authFile}`)
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
      const ownedIdentity = await handle.stat().then(toFileIdentity)
        .catch(() => null)
      await handle.close().catch(() => undefined)
      if (ownedIdentity)
        await removeLockFileIfSame(lockFile, ownedIdentity)
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

/**
 * The auth.json entry Demi will use, with the access token it carries. An
 * entry whose `key` is missing or blank is a login the Grok CLI never
 * finished: the selectors skip it, so a selected entry always has a token.
 */
export interface SelectedGrokAuthEntry {
  entryKey: string
  entry: GrokAuthEntry
  accessToken: string
}

export function selectAuthEntry(
  file: GrokAuthDotJson
): SelectedGrokAuthEntry | null {
  const candidates: Array<SelectedGrokAuthEntry & { score: number }> = []
  for (const [entryKey, entry] of Object.entries(file)) {
    const accessToken = nonEmptyString(entry.key)
    if (!accessToken)
      continue
    let score = 0
    if (entry.auth_mode === 'oidc')
      score += 4
    if (nonEmptyString(entry.refresh_token))
      score += 2
    if (entryKey.includes('auth.x.ai')
      || entry.oidc_issuer === 'https://auth.x.ai') score += 1
    candidates.push({ entryKey, entry, accessToken, score })
  }
  candidates.sort((a, b) => b.score - a.score
    || a.entryKey.localeCompare(b.entryKey))
  const best = candidates[0]
  if (!best)
    return null
  return {
    entryKey: best.entryKey,
    entry: best.entry,
    accessToken: best.accessToken
  }
}

export function selectAuthEntryByKey(
  file: GrokAuthDotJson,
  entryKey: string,
): SelectedGrokAuthEntry | null {
  const entry = file[entryKey]
  const accessToken = entry ? nonEmptyString(entry.key) : undefined
  if (!entry || !accessToken)
    return null
  return { entryKey, entry, accessToken }
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
): Promise<GrokRefreshTokenResponse> {
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
  return grokRefreshTokenResponseSchema.parse(await response.json())
}

/**
 * Decodes stored Grok auth material. `what` names the source in the error a
 * malformed file or payload raises; corrupt material is reported, never
 * repaired.
 */
export function parseGrokAuthDotJson(
  text: string,
  what: string
): GrokAuthDotJson {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    throw new GrokAuthError('auth_invalid', `${what} is not valid JSON`)
  }
  const file = grokAuthDotJsonSchema.safeParse(value)
  if (!file.success) {
    throw new GrokAuthError(
      'auth_invalid',
      `${what} is malformed: ${redactGrokSecretText(z.prettifyError(file.error))}`,
    )
  }
  return file.data
}

export function peekGrokAccessTokenPrincipal(
  accessToken: string
): {
  principalType: string
  principalId: string
  teamId: string | null
} | null {
  const claims = grokJwtClaims(accessToken)
  if (!claims)
    return null
  const principalType = claims.principal_type ?? claims.principalType
  const principalId = claims.principal_id ?? claims.principalId
  if (!principalType || !principalId)
    return null
  return { principalType, principalId, teamId: claims.team_id ?? null }
}

export function parseJwtExpiration(jwt: string): Date | null {
  const exp = grokJwtClaims(jwt)?.exp
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

/**
 * The entry's `expires_at`, or null when it is absent or not a date. Expiry
 * only decides when to refresh early, and the access token's own `exp` is the
 * fallback, so an unreadable timestamp is not a reason to reject the entry.
 */
function parseExpiresAt(value: string | undefined): Date | null {
  if (!value)
    return null
  const ms = Date.parse(value)
  return Number.isFinite(ms) ? new Date(ms) : null
}

function expiresWithin(
  expiresAt: Date | null,
  now: Date,
  skewMs: number
): boolean {
  return expiresAt !== null && expiresAt.getTime() - now.getTime() <= skewMs
}

/**
 * The claims Demi reads from a Grok access or id token. The signature is not
 * verified and the backend is what accepts or rejects the token, so a claim of
 * the wrong type reads as absent rather than failing an otherwise usable
 * login. Grok spells its principal claims in both snake and camel case.
 */
const grokJwtClaimsSchema = z.looseObject({
  exp: z.number().optional().catch(undefined),
  sub: reportedStringSchema,
  email: reportedStringSchema,
  principal_type: reportedStringSchema,
  principalType: reportedStringSchema,
  principal_id: reportedStringSchema,
  principalId: reportedStringSchema,
  team_id: reportedStringSchema,
})

export type GrokJwtClaims = z.infer<typeof grokJwtClaimsSchema>

/** The claims of a Grok JWT, or null when the token carries none Demi reads. */
export function grokJwtClaims(jwt: string): GrokJwtClaims | null {
  const payload = decodeJwtPayload(jwt)
  if (payload === null)
    return null
  return grokJwtClaimsSchema.parse(payload)
}

function resolveGrokUserId(
  entry: GrokAuthEntry,
  accessToken: string
): string | null {
  return (
    entry.user_id ??
    peekGrokAccessTokenPrincipal(accessToken)?.principalId ??
    grokJwtClaims(accessToken)?.sub ??
    null
  )
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
    userId: resolveGrokUserId(entry, accessToken),
    principalType: entry.principal_type ?? peeked?.principalType ?? null,
    principalId: entry.principal_id ?? peeked?.principalId ?? null,
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
  } catch {
    return true
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
  return stat(path).then(toFileIdentity).catch(() => null)
}

async function removeLockFileIfSame(
  lockFile: string,
  expected: FileIdentity
): Promise<boolean> {
  const current = await fileIdentity(lockFile)
  if (!current || current.dev !== expected.dev || current.ino !== expected.ino)
    return false
  return rm(lockFile).then(() => true).catch(() => false)
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

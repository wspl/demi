import type { ProviderAuthState } from '@demicodes/provider'
import { errorCode, isRecord } from '@demicodes/utils'
import { execFile } from 'node:child_process'
import { readFile, writeFile, rename, rm } from 'node:fs/promises'
import { randomUUID } from 'node:crypto'
import { promisify } from 'node:util'
import process from 'node:process'
import {
  ClaudeCodeAuthError, claudeKeychainSchema, claudeOAuthSecretSchema, claudeTokenSchema,
  parseClaudeAuthData, parseClaudeAuthJson,
  type ClaudeCodeOAuthAccess, type ClaudeCodeOAuthSecret,
} from './auth-schemas'
export { ClaudeCodeAuthError } from './auth-schemas'

const execFileAsync = promisify(execFile)

export interface ClaudeCodeAuthStore {
  status(): Promise<ProviderAuthState>
  resolveAccess(
    options?: { forceRefresh?: boolean }
  ): Promise<ClaudeCodeOAuthAccess>
}

/**
 * Renews a pool oauth secret (wired to `refreshClaudeCodeSecret`; injectable in
 * tests).
 */
export type ClaudeCodeSecretRefresh = (
  secret: ClaudeCodeOAuthSecret
) => Promise<unknown>

const OAUTH_EXPIRY_SKEW_MS = 5 * 60 * 1000

export interface FileClaudeCodeAuthStoreOptions {
  /** Optional path to oauth.json (pool entry). */
  oauthFile?: string
  /** Prefer this token over env/keychain when set (tests / static). */
  accessToken?: string | null
  /** Renews the oauth file when its access token nears expiry. */
  refresh?: ClaudeCodeSecretRefresh
  /** Inject external state readers for tests or managed environments. */
  env?: Record<string, string | undefined>
  readKeychain?: () => Promise<string | null>
}

/**
 * Resolves Claude OAuth: explicit token → oauth file → CLAUDE_CODE_OAUTH_TOKEN
 * → keychain.
 */
export class FileClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  private readonly oauthFile: string | null
  private readonly accessToken: string | null
  private readonly refresh: ClaudeCodeSecretRefresh | null
  private readonly env: Record<string, string | undefined>
  private readonly readKeychain: () => Promise<string | null>

  constructor(options: FileClaudeCodeAuthStoreOptions = {}) {
    this.oauthFile = options.oauthFile ?? null
    this.accessToken = parseClaudeAuthData(
      claudeTokenSchema.nullable().optional(), options.accessToken, 'Claude static token',
    ) ?? null
    this.refresh = options.refresh ?? null
    this.env = options.env ?? process.env
    this.readKeychain = options.readKeychain ?? readClaudeKeychain
  }

  async status(): Promise<ProviderAuthState> {
    try {
      const access = await this.resolveAccess()
      return {
        status: 'authenticated',
        accountLabel: access.subscriptionType ?? 'Claude Code',
      }
    } catch (error) {
      if (error instanceof ClaudeCodeAuthError && error.code
        === 'auth_missing') {
        return { status: 'unauthenticated', message: error.message }
      }
      return {
        status: 'error',
        message: error instanceof Error ? error.message : String(error)
      }
    }
  }

  async resolveAccess(options: { forceRefresh?: boolean } = {}): Promise<ClaudeCodeOAuthAccess> {
    if (this.accessToken) {
      return { accessToken: this.accessToken, source: 'static' }
    }
    if (this.oauthFile) {
      let text: string
      try {
        text = await readFile(this.oauthFile, 'utf8')
      } catch (error) {
        if (errorCode(error) === 'ENOENT') {
          throw new ClaudeCodeAuthError('auth_missing', `Claude OAuth file not found: ${this.oauthFile}`)
        }
        throw error
      }
      let secret = parseClaudeAuthJson(claudeOAuthSecretSchema, text, 'Claude OAuth file')
      const isExpiring = secret.expiresAt !== undefined && secret.expiresAt !== null
        && Date.parse(secret.expiresAt) - Date.now() < OAUTH_EXPIRY_SKEW_MS
      if ((options.forceRefresh || isExpiring) && secret.refreshToken && this.refresh) {
        secret = parseClaudeAuthData(claudeOAuthSecretSchema, await this.refresh(secret), 'Claude renewed OAuth secret')
        await writeOAuthSecret(this.oauthFile, secret)
      }
      return {
        accessToken: secret.accessToken, source: 'file',
        subscriptionType: secret.subscriptionType ?? null,
        rateLimitTier: secret.rateLimitTier ?? null,
      }
    }
    const fromEnv = parseClaudeAuthData(
      claudeTokenSchema.optional(), this.env.CLAUDE_CODE_OAUTH_TOKEN, 'Claude environment token',
    )
    if (fromEnv !== undefined) {
      return { accessToken: fromEnv, source: 'env' }
    }
    const keychainText = await this.readKeychain()
    if (keychainText !== null) {
      const { claudeAiOauth: oauth } = parseClaudeAuthJson(claudeKeychainSchema, keychainText, 'Claude keychain item')
      if (oauth) {
        return {
          accessToken: oauth.accessToken, source: 'keychain',
          subscriptionType: oauth.subscriptionType ?? null,
          rateLimitTier: oauth.rateLimitTier ?? null,
        }
      }
    }
    throw new ClaudeCodeAuthError(
      'auth_missing',
      'Claude Code OAuth access token not found (set CLAUDE_CODE_OAUTH_TOKEN or log in with Claude Code)',
    )
  }
}

async function readClaudeKeychain(): Promise<string | null> {
  if (process.platform !== 'darwin') {
    return null
  }
  try {
    const { stdout } = await execFileAsync('security', [
      'find-generic-password', '-s', 'Claude Code-credentials', '-w',
    ], { encoding: 'utf8', timeout: 5_000 })
    return stdout
  } catch (error) {
    // security returns errSecItemNotFound (-25300), whose process exit byte is 44.
    if (isRecord(error) && error.code === 44) {
      return null
    }
    throw error
  }
}

async function writeOAuthSecret(path: string, secret: ClaudeCodeOAuthSecret): Promise<void> {
  const temp = `${path}.${randomUUID()}.tmp`
  try {
    await writeFile(temp, `${JSON.stringify(secret, null, 2)}\n`, { mode: 0o600, flag: 'wx' })
    await rename(temp, path)
  } finally {
    await rm(temp, { force: true })
  }
}

export class StaticClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  constructor(private readonly access: ClaudeCodeOAuthAccess) {}

  async status(): Promise<ProviderAuthState> {
    return {
      status: 'authenticated',
      accountLabel: this.access.subscriptionType ?? 'Claude Code',
    }
  }

  async resolveAccess(): Promise<ClaudeCodeOAuthAccess> {
    return this.access
  }
}

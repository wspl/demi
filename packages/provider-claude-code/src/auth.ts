import type { ProviderAuthState } from '@demicodes/provider'
import { errorMessage } from '@demicodes/utils'
import { execFile } from 'node:child_process'
import { readFile, writeFile } from 'node:fs/promises'
import { promisify } from 'node:util'
import process from 'node:process'
import { z } from 'zod'
import type { ClaudeCodeOAuthAccess } from './oauth'
import {
  claudeCodeOAuthSecretSchema,
  type ClaudeCodeOAuthSecret
} from './secret'

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
) => Promise<ClaudeCodeOAuthSecret>

const OAUTH_EXPIRY_SKEW_MS = 5 * 60 * 1000

/** The credentials the Claude Code CLI keeps in the macOS keychain. */
const keychainCredentialsSchema = z.looseObject({
  claudeAiOauth: z.looseObject({
    accessToken: z.string().min(1),
    subscriptionType: z.string().min(1).optional(),
    rateLimitTier: z.string().min(1).optional(),
  }),
})

export interface FileClaudeCodeAuthStoreOptions {
  /** Optional path to oauth.json (pool entry). */
  oauthFile?: string
  /** Prefer this token over env/keychain when set (tests / static). */
  accessToken?: string | null
  /** Renews the oauth file when its access token nears expiry. */
  refresh?: ClaudeCodeSecretRefresh
}

/**
 * Resolves Claude OAuth: explicit token → oauth file → CLAUDE_CODE_OAUTH_TOKEN
 * → keychain.
 *
 * A credential that is absent moves on to the next source; one that is present
 * but malformed is an `auth_invalid` error, never a silently repaired value.
 */
export class FileClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  private readonly oauthFile: string | null
  private readonly accessToken: string | null
  private readonly refresh: ClaudeCodeSecretRefresh | null

  constructor(options: FileClaudeCodeAuthStoreOptions = {}) {
    this.oauthFile = options.oauthFile ?? null
    this.accessToken = options.accessToken || null
    this.refresh = options.refresh ?? null
  }

  async status(): Promise<ProviderAuthState> {
    try {
      const access = await this.resolveAccess()
      return {
        status: 'authenticated',
        accountLabel: access.subscriptionType || 'Claude Code',
      }
    } catch (error) {
      if (error instanceof ClaudeCodeAuthError && error.code
        === 'auth_missing') {
        return { status: 'unauthenticated', message: error.message }
      }
      return { status: 'error', message: errorMessage(error) }
    }
  }

  async resolveAccess(): Promise<ClaudeCodeOAuthAccess> {
    if (this.accessToken)
      return { accessToken: this.accessToken, source: 'static' }

    if (this.oauthFile)
      return this.accessFromFile(this.oauthFile)

    const fromEnv = process.env.CLAUDE_CODE_OAUTH_TOKEN
    if (fromEnv)
      return { accessToken: fromEnv, source: 'env' }

    if (process.platform === 'darwin') {
      const fromKeychain = await accessFromKeychain()
      if (fromKeychain)
        return fromKeychain
    }

    throw new ClaudeCodeAuthError(
      'auth_missing',
      'Claude Code OAuth access token not found (set CLAUDE_CODE_OAUTH_TOKEN or log in with Claude Code)',
    )
  }

  private async accessFromFile(
    path: string
  ): Promise<ClaudeCodeOAuthAccess> {
    const stored = await readOAuthSecret(path)
    const secret = await this.renewIfExpiring(path, stored)
    return {
      accessToken: secret.accessToken,
      source: 'file',
      subscriptionType: secret.subscriptionType ?? null,
      rateLimitTier: secret.rateLimitTier ?? null,
    }
  }

  /**
   * Renews the secret when its access token is about to expire, and writes the
   * renewed one back. The renewal is validated before it replaces the file, so
   * a bad response cannot corrupt the pool entry.
   */
  private async renewIfExpiring(
    path: string,
    secret: ClaudeCodeOAuthSecret,
  ): Promise<ClaudeCodeOAuthSecret> {
    if (!this.refresh || !secret.refreshToken || !secret.expiresAt)
      return secret
    const remainingMs = Date.parse(secret.expiresAt) - Date.now()
    if (Number.isNaN(remainingMs) || remainingMs >= OAUTH_EXPIRY_SKEW_MS)
      return secret
    const renewed = validateOAuthSecret(
      await this.refresh(secret),
      'Claude OAuth renewal',
    )
    await writeFile(path, `${JSON.stringify(renewed, null, 2)}\n`)
    return renewed
  }
}

export class StaticClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  constructor(private readonly access: ClaudeCodeOAuthAccess) {}

  async status(): Promise<ProviderAuthState> {
    return {
      status: 'authenticated',
      accountLabel: this.access.subscriptionType || 'Claude Code',
    }
  }

  async resolveAccess(): Promise<ClaudeCodeOAuthAccess> {
    return this.access
  }
}

export class ClaudeCodeAuthError extends Error {
  constructor(
    readonly code: 'auth_missing' | 'auth_invalid',
    message: string,
  ) {
    super(message)
    this.name = 'ClaudeCodeAuthError'
  }
}

async function readOAuthSecret(path: string): Promise<ClaudeCodeOAuthSecret> {
  let text: string
  try {
    text = await readFile(path, 'utf8')
  } catch (error) {
    throw new ClaudeCodeAuthError(
      'auth_missing',
      `Failed to read Claude OAuth file ${path}: ${errorMessage(error)}`,
    )
  }
  return validateOAuthSecret(
    parseCredentialJson(text, `Claude OAuth file ${path}`),
    `Claude OAuth file ${path}`,
  )
}

/** Reads the CLI's own keychain item; null when the CLI is not logged in. */
async function accessFromKeychain(): Promise<ClaudeCodeOAuthAccess | null> {
  let stdout: string
  try {
    const result = await execFileAsync(
      'security',
      ['find-generic-password', '-s', 'Claude Code-credentials', '-w'],
      { encoding: 'utf8', timeout: 5_000 },
    )
    stdout = result.stdout
  } catch {
    // No such keychain item, or `security` is unavailable: the caller reports
    // the credential as missing.
    return null
  }

  const source = 'The Claude Code keychain credential'
  const parsed = keychainCredentialsSchema.safeParse(
    parseCredentialJson(stdout.trim(), source)
  )
  if (!parsed.success) {
    throw new ClaudeCodeAuthError(
      'auth_invalid',
      `${source} is invalid: ${z.prettifyError(parsed.error)}`,
    )
  }
  const oauth = parsed.data.claudeAiOauth
  return {
    accessToken: oauth.accessToken,
    source: 'keychain',
    subscriptionType: oauth.subscriptionType ?? null,
    rateLimitTier: oauth.rateLimitTier ?? null,
  }
}

function parseCredentialJson(text: string, source: string): unknown {
  try {
    return JSON.parse(text)
  } catch (error) {
    throw new ClaudeCodeAuthError(
      'auth_invalid',
      `${source} is not valid JSON: ${errorMessage(error)}`,
    )
  }
}

function validateOAuthSecret(
  raw: unknown,
  source: string
): ClaudeCodeOAuthSecret {
  const parsed = claudeCodeOAuthSecretSchema.safeParse(raw)
  if (!parsed.success) {
    throw new ClaudeCodeAuthError(
      'auth_invalid',
      `${source} is invalid: ${z.prettifyError(parsed.error)}`,
    )
  }
  return parsed.data
}

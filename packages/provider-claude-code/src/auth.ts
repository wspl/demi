import type { ProviderAuthState } from '@demicodes/provider'
import { isRecord, nonEmptyString } from '@demicodes/utils'
import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { promisify } from 'node:util'
import process from 'node:process'
import type { ClaudeCodeOAuthAccess } from './oauth'

const execFileAsync = promisify(execFile)

export interface ClaudeCodeAuthStore {
  status(): Promise<ProviderAuthState>
  resolveAccess(options?: { forceRefresh?: boolean }): Promise<ClaudeCodeOAuthAccess>
}

export interface FileClaudeCodeAuthStoreOptions {
  /** Optional path to oauth.json (pool entry). */
  oauthFile?: string
  /** Prefer this token over env/keychain when set (tests / static). */
  accessToken?: string | null
}

/**
 * Resolves Claude OAuth: explicit token → oauth file → CLAUDE_CODE_OAUTH_TOKEN → keychain.
 */
export class FileClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  private readonly oauthFile: string | null
  private readonly accessToken: string | null

  constructor(options: FileClaudeCodeAuthStoreOptions = {}) {
    this.oauthFile = options.oauthFile ?? null
    this.accessToken = nonEmptyString(options.accessToken) ?? null
  }

  async status(): Promise<ProviderAuthState> {
    try {
      const access = await this.resolveAccess()
      return {
        status: 'authenticated',
        accountLabel: nonEmptyString(access.subscriptionType) ?? 'Claude Code',
      }
    } catch (error) {
      if (error instanceof ClaudeCodeAuthError && error.code === 'auth_missing') {
        return { status: 'unauthenticated', message: error.message }
      }
      return { status: 'error', message: error instanceof Error ? error.message : String(error) }
    }
  }

  async resolveAccess(): Promise<ClaudeCodeOAuthAccess> {
    if (this.accessToken) {
      return { accessToken: this.accessToken }
    }
    if (this.oauthFile) {
      try {
        const raw = JSON.parse(await readFile(this.oauthFile, 'utf8')) as unknown
        if (!isRecord(raw)) throw new ClaudeCodeAuthError('auth_invalid', `Invalid OAuth file: ${this.oauthFile}`)
        const accessToken = nonEmptyString(raw.accessToken) ?? nonEmptyString(raw.access_token)
        if (!accessToken) throw new ClaudeCodeAuthError('auth_missing', `No accessToken in ${this.oauthFile}`)
        return {
          accessToken,
          subscriptionType: nonEmptyString(raw.subscriptionType) ?? null,
          rateLimitTier: nonEmptyString(raw.rateLimitTier) ?? null,
        }
      } catch (error) {
        if (error instanceof ClaudeCodeAuthError) throw error
        throw new ClaudeCodeAuthError(
          'auth_missing',
          `Failed to read Claude OAuth file ${this.oauthFile}: ${error instanceof Error ? error.message : String(error)}`,
        )
      }
    }

    const fromEnv = nonEmptyString(process.env.CLAUDE_CODE_OAUTH_TOKEN)
    if (fromEnv) return { accessToken: fromEnv }

    if (process.platform === 'darwin') {
      try {
        const { stdout } = await execFileAsync(
          'security',
          ['find-generic-password', '-s', 'Claude Code-credentials', '-w'],
          { encoding: 'utf8', timeout: 5_000 },
        )
        const parsed = JSON.parse(stdout.trim()) as unknown
        if (!isRecord(parsed)) {
          throw new ClaudeCodeAuthError('auth_missing', 'Claude Code keychain item is not a JSON object')
        }
        const oauth = isRecord(parsed.claudeAiOauth) ? parsed.claudeAiOauth : null
        if (!oauth) throw new ClaudeCodeAuthError('auth_missing', 'Claude Code keychain missing claudeAiOauth')
        const accessToken = nonEmptyString(oauth.accessToken)
        if (!accessToken) throw new ClaudeCodeAuthError('auth_missing', 'Claude Code keychain missing accessToken')
        return {
          accessToken,
          subscriptionType: nonEmptyString(oauth.subscriptionType) ?? null,
          rateLimitTier: nonEmptyString(oauth.rateLimitTier) ?? null,
        }
      } catch (error) {
        if (error instanceof ClaudeCodeAuthError) throw error
        // fall through
      }
    }

    throw new ClaudeCodeAuthError(
      'auth_missing',
      'Claude Code OAuth access token not found (set CLAUDE_CODE_OAUTH_TOKEN or log in with Claude Code)',
    )
  }
}

export class StaticClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  constructor(private readonly access: ClaudeCodeOAuthAccess) {}

  async status(): Promise<ProviderAuthState> {
    return {
      status: 'authenticated',
      accountLabel: nonEmptyString(this.access.subscriptionType) ?? 'Claude Code',
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

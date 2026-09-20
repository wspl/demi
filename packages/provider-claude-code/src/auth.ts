import type { ProviderAuthState } from '@demicodes/provider'
import { errorMessage } from '@demicodes/utils'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import process from 'node:process'
import { z } from 'zod'
import {
  exclusiveCredentialRefresh,
  type CredentialDocument,
} from '@demicodes/provider/credentials-pool'
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
 * Renews an account's oauth secret (wired to `refreshClaudeCodeSecret`; injectable in
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
  /**
   * The account's document in a credential pool. A store with a document
   * stands for that account only: it never reads the environment or the
   * keychain.
   */
  document?: CredentialDocument
  /** Prefer this token over every other source when set (tests / static). */
  accessToken?: string | null
  /** Renews the document when its access token nears expiry. */
  refresh?: ClaudeCodeSecretRefresh
}

/**
 * Resolves Claude OAuth: explicit token → pool document, or without a document
 * the vendor login of this machine: CLAUDE_CODE_OAUTH_TOKEN → keychain.
 *
 * A vendor source that is absent moves on to the next one; a credential that
 * is present but malformed is an `auth_invalid` error, never a silently
 * repaired value.
 */
export class FileClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  private readonly document: CredentialDocument | null
  private readonly accessToken: string | null
  private readonly refresh: ClaudeCodeSecretRefresh | null

  constructor(options: FileClaudeCodeAuthStoreOptions = {}) {
    this.document = options.document ?? null
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

    if (this.document)
      return this.accessFromDocument(this.document)

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

  private async accessFromDocument(
    document: CredentialDocument
  ): Promise<ClaudeCodeOAuthAccess> {
    const stored = (await readRevision(document)).secret
    const secret = this.isExpiring(stored)
      ? await this.renew(document)
      : stored
    return {
      accessToken: secret.accessToken,
      source: 'file',
      subscriptionType: secret.subscriptionType ?? null,
      rateLimitTier: secret.rateLimitTier ?? null,
    }
  }

  /** Whether the secret can be renewed and its access token is about to expire. */
  private isExpiring(secret: ClaudeCodeOAuthSecret): boolean {
    if (!this.refresh || !secret.refreshToken || !secret.expiresAt)
      return false
    const remainingMs = Date.parse(secret.expiresAt) - Date.now()
    return !Number.isNaN(remainingMs) && remainingMs < OAUTH_EXPIRY_SKEW_MS
  }

  /**
   * Renews the document's secret and stores the renewed one. The renewal is
   * validated before it replaces the document, so a bad response cannot
   * corrupt the account.
   */
  private renew(document: CredentialDocument): Promise<ClaudeCodeOAuthSecret> {
    return exclusiveCredentialRefresh(document, async () => {
      const latest = await readRevision(document)
      // An earlier renewal in line has already stored fresh tokens.
      if (!this.refresh || !this.isExpiring(latest.secret))
        return latest.secret
      let renewed: ClaudeCodeOAuthSecret
      try {
        renewed = validateOAuthSecret(
          await this.refresh(latest.secret),
          'Claude OAuth renewal',
        )
      } catch (error) {
        // A refresh token is spent once: whoever renewed first has stored
        // the tokens this one was refused for.
        const stored = await readRevision(document)
        if (stored.version !== latest.version)
          return stored.secret
        throw error
      }
      const kept = await document.replace(
        `${JSON.stringify(renewed, null, 2)}\n`,
        latest.version
      )
      return kept ? renewed : (await readRevision(document)).secret
    })
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
    readonly code: 'auth_missing' | 'auth_invalid' | 'auth_unsupported',
    message: string,
  ) {
    super(message)
    this.name = 'ClaudeCodeAuthError'
  }
}

async function readRevision(
  document: CredentialDocument
): Promise<{ secret: ClaudeCodeOAuthSecret; version: string }> {
  const revision = await document.read()
  if (!revision)
    throw new ClaudeCodeAuthError(
      'auth_missing',
      `Claude OAuth credential not found: ${document.name}`,
    )
  const source = `Claude OAuth credential ${document.name}`
  return {
    secret: validateOAuthSecret(
      parseCredentialJson(revision.text, source),
      source
    ),
    version: revision.version,
  }
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

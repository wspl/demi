import type { ProviderAuthState } from '@demicodes/provider'
import { errorMessage } from '@demicodes/utils'
import { z } from 'zod'
import type { CredentialDocument } from '@demicodes/provider/credentials-pool'
import type { ClaudeCodeOAuthAccess, ClaudeCodeOAuthSource } from './oauth'
import {
  claudeCodeOAuthSecretSchema,
  type ClaudeCodeOAuthSecret
} from './secret'

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

export interface ClaudeCodeDocumentAuthStoreOptions {
  /** The account's oauth secret, wherever it is kept. */
  document: CredentialDocument
  /** What the resolved access reports as its source; the account's keeper knows. */
  source: ClaudeCodeOAuthSource
  /** Renews the secret when its access token nears expiry. */
  refresh: ClaudeCodeSecretRefresh
}

/**
 * Resolves and renews one account from its document. The document's keeper
 * decides where the secret lives and who else may renew it; this store never
 * does. A secret without a refresh token is used as it is.
 *
 * A document that is absent is `auth_missing`; one that is present but
 * malformed is an `auth_invalid` error, never a silently repaired value.
 */
export class ClaudeCodeDocumentAuthStore implements ClaudeCodeAuthStore {
  private readonly document: CredentialDocument
  private readonly source: ClaudeCodeOAuthSource
  private readonly refresh: ClaudeCodeSecretRefresh

  constructor(options: ClaudeCodeDocumentAuthStoreOptions) {
    this.document = options.document
    this.source = options.source
    this.refresh = options.refresh
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
    const stored = (await this.readRevision()).secret
    const secret = isExpiring(stored) ? await this.renew() : stored
    return {
      accessToken: secret.accessToken,
      source: this.source,
      subscriptionType: secret.subscriptionType ?? null,
      rateLimitTier: secret.rateLimitTier ?? null,
    }
  }

  /**
   * Renews the document's secret and stores the renewed one. The renewal is
   * validated before it replaces the document, so a bad response cannot
   * corrupt the account.
   */
  private renew(): Promise<ClaudeCodeOAuthSecret> {
    return this.document.exclusive(async () => {
      const latest = await this.readRevision()
      // An earlier renewal in line has already stored fresh tokens.
      if (!isExpiring(latest.secret))
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
        const stored = await this.readRevision()
        if (stored.version !== latest.version)
          return stored.secret
        throw error
      }
      const kept = await this.document.replace(
        `${JSON.stringify(renewed, null, 2)}\n`,
        latest.version
      )
      return kept ? renewed : (await this.readRevision()).secret
    })
  }

  private async readRevision(): Promise<{
    secret: ClaudeCodeOAuthSecret
    version: string
  }> {
    const revision = await this.document.read()
    if (!revision)
      throw new ClaudeCodeAuthError(
        'auth_missing',
        `Claude OAuth credential not found: ${this.document.name}`,
      )
    return {
      secret: parseClaudeCodeOAuthSecret(
        revision.text,
        `Claude OAuth credential ${this.document.name}`
      ),
      version: revision.version,
    }
  }
}

/** Whether the secret can be renewed and its access token is about to expire. */
function isExpiring(secret: ClaudeCodeOAuthSecret): boolean {
  if (!secret.refreshToken || !secret.expiresAt)
    return false
  const remainingMs = Date.parse(secret.expiresAt) - Date.now()
  return !Number.isNaN(remainingMs) && remainingMs < OAUTH_EXPIRY_SKEW_MS
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

/** Parses JSON a credential keeper holds; `source` names it in the error. */
export function parseCredentialJson(text: string, source: string): unknown {
  try {
    return JSON.parse(text)
  } catch (error) {
    throw new ClaudeCodeAuthError(
      'auth_invalid',
      `${source} is not valid JSON: ${errorMessage(error)}`,
    )
  }
}

/** The kit's oauth secret held in `text`; `source` names it in the error. */
export function parseClaudeCodeOAuthSecret(
  text: string,
  source: string
): ClaudeCodeOAuthSecret {
  return validateOAuthSecret(parseCredentialJson(text, source), source)
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

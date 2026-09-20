/**
 * The Claude Code login of this machine, as a credential pool of up to two
 * accounts: the `CLAUDE_CODE_OAUTH_TOKEN` environment token, then the item the
 * CLI keeps in the macOS keychain. Demi reads them and never renews, adds to or
 * removes from them: the CLI owns the keychain login's renewal.
 */
import {
  CredentialPoolError,
  type CredentialDocument,
  type CredentialEntryMeta,
  type CredentialPool,
} from '@demicodes/provider/credentials-pool'
import { nonEmptyString } from '@demicodes/utils'
import { execFile } from 'node:child_process'
import { createHash } from 'node:crypto'
import process from 'node:process'
import { promisify } from 'node:util'
import { z } from 'zod'
import {
  ClaudeCodeAuthError,
  parseClaudeCodeOAuthSecret,
  parseCredentialJson,
} from './auth'
import type { ClaudeCodeOAuthSource } from './oauth'
import type { ClaudeCodeOAuthSecret } from './secret'

const execFileAsync = promisify(execFile)

/** The vendor pool's account for the environment token. */
export const CLAUDE_CODE_ENV_CREDENTIAL_ID = 'env'
/** The vendor pool's account for the CLI's keychain item. */
export const CLAUDE_CODE_KEYCHAIN_CREDENTIAL_ID = 'keychain'

const ENV_SOURCE = 'vendor:env'
const KEYCHAIN_SOURCE = 'vendor:keychain'

/**
 * The source an access reports for an account whose meta names `metaSource`.
 * Only the vendor pool's own accounts are the environment's or the keychain's;
 * an account any other keeper holds is a stored secret.
 */
export function claudeCodeAccessSource(
  metaSource: string | null | undefined
): ClaudeCodeOAuthSource {
  if (metaSource === ENV_SOURCE)
    return 'env'
  if (metaSource === KEYCHAIN_SOURCE)
    return 'keychain'
  return 'file'
}

export interface ClaudeCodeVendorOptions {
  /** The environment the token is read from; by default the process's. */
  env?: Record<string, string | undefined>
  /**
   * Reads the text of the CLI's keychain item, null when there is none; by
   * default the macOS keychain.
   */
  readKeychain?: () => Promise<string | null>
}

/** The credentials the Claude Code CLI keeps in the macOS keychain. */
const keychainCredentialsSchema = z.looseObject({
  claudeAiOauth: z.looseObject({
    accessToken: z.string().min(1),
    subscriptionType: z.string().min(1).optional(),
    rateLimitTier: z.string().min(1).optional(),
  }),
})

const KEYCHAIN_READ_TTL_MS = 2_000
let keychainRead: { at: number; text: Promise<string | null> } | null = null

/**
 * The CLI's own keychain item; null when the CLI is not logged in or this is
 * not macOS. `security` is a subprocess, so one read answers the questions a
 * pool is asked about one request.
 */
function readMacKeychain(): Promise<string | null> {
  if (process.platform !== 'darwin')
    return Promise.resolve(null)
  if (keychainRead && Date.now() - keychainRead.at < KEYCHAIN_READ_TTL_MS)
    return keychainRead.text
  const text = execFileAsync(
    'security',
    ['find-generic-password', '-s', 'Claude Code-credentials', '-w'],
    { encoding: 'utf8', timeout: 5_000 },
  ).then(
    (result) => result.stdout.trim(),
    // No such keychain item, or `security` is unavailable: no account.
    () => null
  )
  keychainRead = { at: Date.now(), text }
  return text
}

/**
 * An oauth secret that is a token and nothing to renew it with: it has no
 * refresh token, so the auth store never renews it.
 */
export type ClaudeCodeTokenSecret = Pick<
  ClaudeCodeOAuthSecret,
  'accessToken' | 'subscriptionType' | 'rateLimitTier'
>

function secretFromKeychainItem(text: string): ClaudeCodeTokenSecret {
  const source = 'The Claude Code keychain credential'
  const parsed = keychainCredentialsSchema.safeParse(
    parseCredentialJson(text, source)
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
    subscriptionType: oauth.subscriptionType ?? null,
    rateLimitTier: oauth.rateLimitTier ?? null,
  }
}

/** A document nobody replaces: its text is built from what `secret` finds. */
function readOnlyDocument(
  name: string,
  secret: () => Promise<ClaudeCodeTokenSecret | null>
): CredentialDocument {
  return {
    name,
    read: async () => {
      const found = await secret()
      if (!found)
        return null
      const text = `${JSON.stringify(found, null, 2)}\n`
      return { text, version: text }
    },
    replace: async () => false,
    exclusive: (fn) => fn(),
  }
}

/** The vendor login of this machine as a pool of at most two accounts. */
export function claudeCodeVendorPool(
  options: ClaudeCodeVendorOptions = {}
): CredentialPool {
  const env = options.env ?? process.env
  const readKeychain = options.readKeychain ?? readMacKeychain
  // In the order they stand in for the login: the first one held is active.
  const accounts = [
    {
      id: CLAUDE_CODE_ENV_CREDENTIAL_ID,
      source: ENV_SOURCE,
      document: readOnlyDocument('CLAUDE_CODE_OAUTH_TOKEN', async () => {
        const token = nonEmptyString(env.CLAUDE_CODE_OAUTH_TOKEN)
        return token ? { accessToken: token } : null
      }),
    },
    {
      id: CLAUDE_CODE_KEYCHAIN_CREDENTIAL_ID,
      source: KEYCHAIN_SOURCE,
      document: readOnlyDocument('the Claude Code keychain item', async () => {
        const text = await readKeychain()
        return text ? secretFromKeychainItem(text) : null
      }),
    },
  ]
  const missing: CredentialDocument = readOnlyDocument(
    'no Claude Code login',
    async () => null
  )

  const metaOf = async (
    account: typeof accounts[number]
  ): Promise<CredentialEntryMeta | null> => {
    const revision = await account.document.read()
    if (!revision)
      return null
    const secret = parseClaudeCodeOAuthSecret(
      revision.text,
      account.document.name
    )
    const { label, identityKey, detail } = labelFromClaudeCodeToken(secret)
    return {
      id: account.id,
      label,
      detail,
      identityKey,
      source: account.source,
      updatedAt: new Date().toISOString(),
    }
  }
  const held = async (id: string): Promise<CredentialEntryMeta | null> => {
    const account = accounts.find((a) => a.id === id)
    return account ? metaOf(account) : null
  }
  const listMeta = async (): Promise<CredentialEntryMeta[]> => {
    const out: CredentialEntryMeta[] = []
    for (const account of accounts) {
      const meta = await metaOf(account)
      if (meta)
        out.push(meta)
    }
    return out
  }
  const activeId = async (): Promise<string | null> => {
    for (const account of accounts) {
      if (await account.document.read())
        return account.id
    }
    return null
  }
  const refuse = (): never => {
    throw new CredentialPoolError(
      'credential_invalid',
      'The Claude Code login is the vendor\'s to change: set CLAUDE_CODE_OAUTH_TOKEN or run `claude auth login`'
    )
  }
  return {
    list: async () => (await listMeta()).map((m) => ({
      id: m.id,
      label: m.label,
      detail: m.detail ?? null,
      updatedAt: m.updatedAt,
    })),
    listMeta,
    readMeta: held,
    findByIdentityKey: async (identityKey) =>
      (await listMeta()).find((m) => m.identityKey === identityKey) ?? null,
    getActiveId: activeId,
    setActiveId: async (id) => {
      if (!await held(id))
        throw new CredentialPoolError(
          'credential_not_found',
          `Credential "${id}" not found`
        )
    },
    ensureActivePointer: activeId,
    writeEntry: async () => refuse(),
    document: (id) =>
      accounts.find((a) => a.id === id)?.document ?? missing,
    remove: async () => refuse(),
  }
}

/** How a token names itself: its label, and the key that tells accounts apart. */
export function labelFromClaudeCodeToken(secret: ClaudeCodeTokenSecret): {
  label: string
  identityKey: string
  detail: string | null
} {
  const hash = createHash('sha256')
    .update(secret.accessToken)
    .digest('hex')
    .slice(0, 16)
  const subscriptionType = nonEmptyString(secret.subscriptionType)
  const identityKey = subscriptionType
    ? `token:${hash}:${subscriptionType}`
    : `token:${hash}`
  return {
    label: subscriptionType ?? `claude-${identityKey.slice(-8)}`,
    identityKey,
    detail: nonEmptyString(secret.rateLimitTier) ?? null,
  }
}

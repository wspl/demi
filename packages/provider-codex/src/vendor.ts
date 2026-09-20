/**
 * The Codex CLI's own login on this machine, as a credential pool: one account,
 * `auth.json` in the Codex home, refreshed under the CLI's lock file so the CLI
 * and Demi do not refresh at once. Demi reads and refreshes it and never adds
 * to or removes from it.
 */
import {
  CredentialPoolError,
  type CredentialDocument,
  type CredentialEntryMeta,
  type CredentialPool,
} from '@demicodes/provider/credentials-pool'
import type { ProviderAuthState } from '@demicodes/provider'
import { delay, errorCode, errorMessage, nonEmptyString } from '@demicodes/utils'
import { randomUUID } from 'node:crypto'
import { chmod, mkdir, open, readFile, rename, rm, stat, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import {
  CodexAuthError,
  CodexDocumentAuthStore,
  defaultCodexHome,
  parseChatGptClaims,
  parseCodexAuthDotJson,
  parseIdTokenClaims,
  redactCodexSecretText,
  type CodexAuthDotJson,
  type CodexTokenRefresh,
} from './auth'

/** The vendor pool's one account. */
export const CODEX_VENDOR_CREDENTIAL_ID = 'vendor'

export interface CodexVendorOptions {
  codexHome?: string
  /** Override the `auth.json` path. */
  authFile?: string
  lockRetryDelayMs?: number
  lockTimeoutMs?: number
}

function authFileOf(options: CodexVendorOptions): string {
  return options.authFile
    ?? join(options.codexHome ?? defaultCodexHome(), 'auth.json')
}

/** The vendor's `auth.json` as an account document. */
export function codexVendorDocument(
  options: CodexVendorOptions = {}
): CredentialDocument {
  const authFile = authFileOf(options)
  const lockRetryDelayMs = options.lockRetryDelayMs ?? 25
  const lockTimeoutMs = options.lockTimeoutMs ?? 5_000
  const read = async () => {
    try {
      const text = await readFile(authFile, 'utf8')
      // The lock keeps other writers out; the text tells revisions apart.
      return { text, version: text }
    } catch (error) {
      if (errorCode(error) === 'ENOENT')
        return null
      throw new CodexAuthError(
        'auth_invalid',
        `Failed to read Codex auth file ${authFile}: ${redactCodexSecretText(errorMessage(error))}`
      )
    }
  }
  return {
    name: authFile,
    read,
    replace: async (text, version) => {
      if ((await read())?.version !== version)
        return false
      await mkdir(dirname(authFile), { recursive: true })
      const temp = `${authFile}.${process.pid}.${randomUUID().slice(0, 8)}.tmp`
      await writeFile(temp, text, { mode: 0o600 })
      await chmod(temp, 0o600)
      await rename(temp, authFile)
      return true
    },
    exclusive: async (fn) => {
      const lockFile = `${authFile}.lock`
      await mkdir(dirname(authFile), { recursive: true })
      const started = Date.now()
      let handle: Awaited<ReturnType<typeof open>> | null = null
      while (!handle) {
        try {
          handle = await open(lockFile, 'wx', 0o600)
        } catch (error) {
          if (errorCode(error) !== 'EEXIST'
            || Date.now() - started > lockTimeoutMs) {
            throw new CodexAuthError(
              'auth_lock_failed',
              `Failed to lock Codex auth file: ${redactCodexSecretText(errorMessage(error))}`
            )
          }
          await delay(lockRetryDelayMs)
        }
      }
      try {
        return await fn()
      } finally {
        await handle.close().catch(() => undefined)
        await rm(lockFile, { force: true }).catch(() => undefined)
      }
    },
  }
}

/** The vendor login of this machine as a pool of at most one account. */
export function codexVendorPool(
  options: CodexVendorOptions = {}
): CredentialPool {
  const authFile = authFileOf(options)
  const document = codexVendorDocument(options)
  const meta = async (): Promise<CredentialEntryMeta | null> => {
    const revision = await document.read()
    if (!revision)
      return null
    const auth = parseCodexAuthDotJson(revision.text, `Codex auth file ${authFile}`)
    const { label, identityKey, detail } = labelFromCodexAuth(auth)
    return {
      id: CODEX_VENDOR_CREDENTIAL_ID,
      label,
      detail,
      identityKey,
      source: `vendor:${authFile}`,
      updatedAt: (await stat(authFile)).mtime.toISOString(),
    }
  }
  const held = async (id: string) =>
    id === CODEX_VENDOR_CREDENTIAL_ID ? meta() : null
  const refuse = (): never => {
    throw new CredentialPoolError(
      'credential_invalid',
      'The Codex CLI login is the vendor\'s to change: run `codex login` or `codex logout`'
    )
  }
  return {
    list: async () => (await meta().then(m => m ? [m] : [])).map(m => ({
      id: m.id,
      label: m.label,
      detail: m.detail ?? null,
      updatedAt: m.updatedAt,
    })),
    listMeta: async () => meta().then(m => m ? [m] : []),
    readMeta: held,
    findByIdentityKey: async (identityKey) => {
      const m = await meta()
      return m?.identityKey === identityKey ? m : null
    },
    getActiveId: async () => (await meta())?.id ?? null,
    setActiveId: async (id) => {
      if (!await held(id))
        throw new CredentialPoolError(
          'credential_not_found',
          `Credential "${id}" not found`
        )
    },
    ensureActivePointer: async () => (await meta())?.id ?? null,
    writeEntry: async () => refuse(),
    document: (id) => id === CODEX_VENDOR_CREDENTIAL_ID
      ? document
      : { ...document, read: async () => null, replace: async () => false },
    remove: async () => refuse(),
  }
}

export interface FileCodexAuthStoreOptions extends CodexVendorOptions {
  refresh?: CodexTokenRefresh
  now?: () => Date
}

/** The auth store of the vendor's own `auth.json`. */
export class FileCodexAuthStore extends CodexDocumentAuthStore {
  constructor(options: FileCodexAuthStoreOptions = {}) {
    super({
      document: codexVendorDocument(options),
      refresh: options.refresh,
      now: options.now,
    })
  }
}

export async function codexAuthStatus(
  options: FileCodexAuthStoreOptions = {}
): Promise<ProviderAuthState> {
  return new FileCodexAuthStore(options).status()
}

/** How an account names itself: its label, and the key that tells accounts apart. */
export function labelFromCodexAuth(
  auth: CodexAuthDotJson
): {
  label: string;
  identityKey: string | null;
  detail: string | null
} {
  if (nonEmptyString(auth.OPENAI_API_KEY)) {
    return { label: 'OPENAI_API_KEY', identityKey: 'apiKey', detail: 'apiKey' }
  }
  const pat = nonEmptyString(auth.personal_access_token)
  if (pat) {
    const claims = parseChatGptClaims(pat)
    const label = claims.email ?? claims.accountId ?? 'personal access token'
    return {
      label,
      identityKey: claims.accountId ?? label,
      detail: 'personalAccessToken'
    }
  }
  const tokens = auth.tokens
  if (tokens) {
    const access = nonEmptyString(tokens.access_token)
    const idClaims = parseIdTokenClaims(tokens.id_token)
    const accessClaims = access ? parseChatGptClaims(access) : {
      accountId: null,
      email: null,
      isFedrampAccount: false
    }
    const accountId = nonEmptyString(tokens.account_id) ?? idClaims.accountId
      ?? accessClaims.accountId
    const email = idClaims.email ?? accessClaims.email
    const label = email ?? accountId ?? 'chatgpt'
    return { label, identityKey: accountId ?? email, detail: 'chatgpt' }
  }
  return { label: 'codex', identityKey: null, detail: null }
}

/**
 * The Grok CLI's own logins on this machine, as a credential pool: one account
 * per entry of `auth.json` in the Grok home, all kept in that one file and
 * refreshed under the CLI's lock file so the CLI and Demi do not refresh at
 * once. Demi reads and refreshes them and never adds to or removes from them.
 */
import {
  CredentialPoolError,
  type CredentialDocument,
  type CredentialEntryMeta,
  type CredentialPool,
} from '@demicodes/provider/credentials-pool'
import type { ProviderAuthState } from '@demicodes/provider'
import { delay, errorCode, errorMessage, nonEmptyString } from '@demicodes/utils'
import { createHash, randomUUID } from 'node:crypto'
import { chmod, mkdir, open, readFile, rename, rm, stat, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import process from 'node:process'
import {
  GrokAuthError,
  GrokDocumentAuthStore,
  defaultGrokHome,
  parseGrokAuthDotJson,
  redactGrokSecretText,
  selectAuthEntry,
  type GrokAuthEntry,
  type GrokTokenRefresh,
} from './auth'

export interface GrokVendorOptions {
  grokHome?: string
  /** Override the `auth.json` path. */
  authFile?: string
  /** The clock the lock file's timestamp and age are read against. */
  now?: () => Date
  lockRetryDelayMs?: number
  lockTimeoutMs?: number
}

function authFileOf(options: GrokVendorOptions): string {
  return options.authFile
    ?? join(options.grokHome ?? defaultGrokHome(), 'auth.json')
}

/** The vendor's `auth.json` as an account document. */
export function grokVendorDocument(
  options: GrokVendorOptions = {}
): CredentialDocument {
  const authFile = authFileOf(options)
  const now = options.now ?? (() => new Date())
  const lockRetryDelayMs = options.lockRetryDelayMs ?? 25
  // Covers the lock holder's whole token refresh (a network round-trip), so a
  // contender waits for the result instead of failing.
  const lockTimeoutMs = options.lockTimeoutMs ?? 30_000
  const read = async () => {
    try {
      const text = await readFile(authFile, 'utf8')
      // The lock keeps other writers out; the text tells revisions apart.
      return { text, version: text }
    } catch (error) {
      if (errorCode(error) === 'ENOENT')
        return null
      throw new GrokAuthError(
        'auth_invalid',
        `Failed to read Grok auth file ${authFile}: ${redactGrokSecretText(errorMessage(error))}`
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
          // The Grok CLI writes `auth.json.lock` as `pid:unix_ts` and may leave
          // it behind after a crash. Only an abandoned lock is stolen, and only
          // once; a lock a live process holds is waited for.
          const staleIdentity = brokeStaleLock
            ? null
            : await fileIdentity(lockFile)
          if (staleIdentity && (await isAbandonedGrokAuthLock(lockFile, now()))) {
            if (await removeLockFileIfSame(lockFile, staleIdentity))
              brokeStaleLock = true
            continue
          }
          if (Date.now() - started > lockTimeoutMs) {
            throw new GrokAuthError(
              'auth_lock_failed',
              `Timed out waiting for Grok auth lock ${lockFile}. If no other Grok process is running, delete the lock file and retry.`,
            )
          }
          await delay(lockRetryDelayMs)
        }
      }

      try {
        // The Grok CLI's lock payload, so concurrent tools can tell who holds it.
        await writeFile(
          lockFile,
          `${process.pid}:${Math.floor(now().getTime() / 1000)}`,
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
    },
  }
}

/** The id of the vendor account kept under `entryKey`. */
export function grokVendorCredentialId(entryKey: string): string {
  const hash = createHash('sha256').update(entryKey).digest('hex').slice(0, 16)
  return `vendor-${hash}`
}

/**
 * The vendor logins of this machine as a pool: every entry of `auth.json` that
 * carries an access token is an account, and the one in use is the entry the
 * Grok CLI's own preference picks.
 */
export function grokVendorPool(
  options: GrokVendorOptions = {}
): CredentialPool {
  const authFile = authFileOf(options)
  const document = grokVendorDocument(options)
  const accounts = async (): Promise<{
    all: CredentialEntryMeta[]
    activeId: string | null
  }> => {
    const revision = await document.read()
    if (!revision)
      return { all: [], activeId: null }
    const file = parseGrokAuthDotJson(revision.text, `Grok auth file ${authFile}`)
    const updatedAt = (await stat(authFile)).mtime.toISOString()
    const all = Object.entries(file)
      .filter(([, entry]) => nonEmptyString(entry.key))
      .map(([entryKey, entry]): CredentialEntryMeta => ({
        id: grokVendorCredentialId(entryKey),
        ...labelFromGrokEntry(entryKey, entry),
        source: `vendor:${authFile}`,
        updatedAt,
      }))
      .sort((a, b) => a.id.localeCompare(b.id))
    const preferred = selectAuthEntry(file)
    return {
      all,
      activeId: preferred ? grokVendorCredentialId(preferred.entryKey) : null,
    }
  }
  const held = async (id: string) =>
    (await accounts()).all.find((m) => m.id === id) ?? null
  const activeId = async () => (await accounts()).activeId
  const refuse = (): never => {
    throw new CredentialPoolError(
      'credential_invalid',
      'The Grok CLI logins are the vendor\'s to change: run `grok login` or `grok logout`'
    )
  }
  return {
    list: async () => (await accounts()).all.map((m) => ({
      id: m.id,
      label: m.label,
      detail: m.detail ?? null,
      updatedAt: m.updatedAt,
    })),
    listMeta: async () => (await accounts()).all,
    readMeta: held,
    findByIdentityKey: async (identityKey) =>
      (await accounts()).all.find((m) => m.identityKey === identityKey) ?? null,
    getActiveId: activeId,
    setActiveId: async (id) => {
      if (!await held(id))
        throw new CredentialPoolError(
          'credential_not_found',
          `Credential "${id}" not found`
        )
      // The file has no pointer to move: which login is in use is read off it.
      if (id !== await activeId())
        refuse()
    },
    ensureActivePointer: activeId,
    writeEntry: async () => refuse(),
    document: (id) => ({
      ...document,
      read: async () => (await held(id)) ? document.read() : null,
      replace: async (text, version) =>
        (await held(id)) ? document.replace(text, version) : false,
    }),
    remove: async () => refuse(),
  }
}

export interface FileGrokAuthStoreOptions extends GrokVendorOptions {
  /**
   * The map key of the login to use when the file holds several.
   * When unset, uses {@link selectAuthEntry} scoring.
   */
  entryKey?: string
  refresh?: GrokTokenRefresh
}

/** The auth store of the vendor's own `auth.json`. */
export class FileGrokAuthStore extends GrokDocumentAuthStore {
  constructor(options: FileGrokAuthStoreOptions = {}) {
    super({
      document: grokVendorDocument(options),
      entryKey: options.entryKey,
      refresh: options.refresh,
      now: options.now,
    })
  }
}

export async function grokBuildAuthStatus(
  options: FileGrokAuthStoreOptions = {}
): Promise<ProviderAuthState> {
  return new FileGrokAuthStore(options).status()
}

/**
 * How an account names itself: its label, and the key that tells accounts
 * apart, which is the key its entry is kept under in an auth map.
 */
export function labelFromGrokEntry(
  entryKey: string,
  entry: GrokAuthEntry
): {
  label: string
  identityKey: string
  detail: string
} {
  return {
    label: nonEmptyString(entry.email) ?? entryKey,
    identityKey: entryKey,
    detail: nonEmptyString(entry.auth_mode) ?? 'oidc',
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

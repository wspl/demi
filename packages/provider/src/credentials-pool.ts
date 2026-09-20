/**
 * The accounts of one provider entry, wherever they are kept. The framework's
 * own pool is on disk:
 * <stateDir>/credentials/<providerKey>/{active,entries/<id>/{meta.json,secret}}
 */
import {
  errorCode,
  errorMessage,
  isFileNotFoundError,
  nonEmptyString
} from '@demicodes/utils'
import { createHash, randomUUID } from 'node:crypto'
import {
  chmod,
  mkdir,
  readdir,
  readFile,
  rename,
  rm,
  stat,
  writeFile
} from 'node:fs/promises'
import { homedir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { z } from 'zod'
import type { ProviderCredentialInfo } from './types'

/**
 * An entry's `meta.json`, as this pool writes it. The pool owns the file, so a
 * file that does not match is corrupt state, not an older shape to repair.
 */
export const credentialEntryMetaSchema = z.looseObject({
  id: z.string().min(1),
  label: z.string().min(1),
  detail: z.string().min(1).nullable().optional(),
  updatedAt: z.iso.datetime(),
  source: z.string().min(1).nullable().optional(),
  /**
   * Stable account key for upsert on re-import (email, accountId, entryKey, …).
   */
  identityKey: z.string().min(1).nullable().optional(),
})
export type CredentialEntryMeta = z.infer<typeof credentialEntryMetaSchema>

/** One revision of an account's secret document. */
export interface CredentialDocumentRevision {
  text: string
  /** Changes whenever the text does; only ever compared for equality. */
  version: string
}

/**
 * One account's secret document. Its text is the provider kit's own format
 * (an `auth.json`, OAuth tokens); the keeper never looks inside.
 */
export interface CredentialDocument {
  /** Identifies the document across every handle to it. Never secret. */
  readonly key: string
  /** Names the document in messages. Never secret. */
  readonly name: string
  read(): Promise<CredentialDocumentRevision | null>
  /**
   * Stores `text` if the document is still at `version`. False when someone
   * else wrote first: the caller reads again and uses what it finds.
   */
  replace(text: string, version: string): Promise<boolean>
}

/**
 * The accounts of one provider entry and which of them is in use. A kit reads
 * and writes accounts only through this.
 */
export interface CredentialPool {
  /**
   * Whether an empty pool stands for the vendor's own login on this machine.
   * True only where the machine is the user's: a product serving other users
   * must not lend them the login of whoever operates it.
   */
  readonly vendorDefault: boolean
  list(): Promise<ProviderCredentialInfo[]>
  listMeta(): Promise<CredentialEntryMeta[]>
  readMeta(id: string): Promise<CredentialEntryMeta | null>
  findByIdentityKey(identityKey: string): Promise<CredentialEntryMeta | null>
  getActiveId(): Promise<string | null>
  setActiveId(id: string): Promise<void>
  /** The active id, choosing the first account when none is chosen. */
  ensureActivePointer(): Promise<string | null>
  writeEntry(meta: CredentialEntryMeta, secretText: string): Promise<CredentialEntryMeta>
  document(id: string): CredentialDocument
  remove(id: string): Promise<void>
}

const refreshes = new Map<string, Promise<unknown>>()

/**
 * Runs `fn` after every earlier refresh of the same document in this process:
 * a refresh token is spent once, so refreshes of one account take turns and
 * the later one finds the earlier one's tokens.
 */
export function exclusiveCredentialRefresh<T>(
  document: CredentialDocument,
  fn: () => Promise<T>
): Promise<T> {
  const previous = refreshes.get(document.key) ?? Promise.resolve()
  const run = previous.then(fn, fn)
  const settled = run.catch(() => undefined)
  refreshes.set(document.key, settled)
  void settled.then(() => {
    if (refreshes.get(document.key) === settled)
      refreshes.delete(document.key)
  })
  return run
}

function textVersion(text: string): string {
  return createHash('sha256').update(text).digest('hex')
}

export interface FileCredentialPoolOptions {
  /** Demi state root ($DEMI_HOME / ~/.demi). */
  stateDir?: string
  /** Subdir under credentials/ (e.g. codex, grok-build, claude-code). */
  providerKey: string
  /** Secret filename inside each entry (auth.json, oauth.json). */
  secretFileName: string
}

/**
 * Demi local state root (`$DEMI_HOME` / `~/.demi`). Canonical copy — the runner
 * re-exports this for its bridge layout.
 */
export function resolveDemiHome(explicit?: string): string {
  if (explicit && explicit.trim())
    return resolve(explicit.trim())
  const fromEnv = process.env.DEMI_HOME
  if (fromEnv && fromEnv.trim())
    return resolve(fromEnv.trim())
  return join(homedir(), '.demi')
}

export function credentialIdFromIdentity(
  identityKey: string | null | undefined,
  fallbackLabel: string
): string {
  const basis = nonEmptyString(identityKey) ?? fallbackLabel
  const hash = createHash('sha256').update(basis).digest('hex').slice(0, 16)
  return `cred-${hash}`
}

export function newCredentialId(): string {
  return `cred-${randomUUID().replace(/-/g, '').slice(0, 16)}`
}

/**
 * Reports whether a read failed because the pool holds nothing there: the file
 * is absent (`ENOENT`), or the path Demi expected to be an entry directory is
 * a plain file (`ENOTDIR`, e.g. a `.DS_Store` beside the entries).
 */
function isMissingEntryError(error: unknown): boolean {
  return isFileNotFoundError(error) || errorCode(error) === 'ENOTDIR'
}

export class FileCredentialPool implements CredentialPool {
  readonly vendorDefault = true
  readonly root: string
  readonly secretFileName: string

  constructor(options: FileCredentialPoolOptions) {
    const stateDir = resolveDemiHome(options.stateDir)
    this.root = join(stateDir, 'credentials', options.providerKey)
    this.secretFileName = options.secretFileName
  }

  entriesDir(): string {
    return join(this.root, 'entries')
  }

  entryDir(id: string): string {
    return join(this.entriesDir(), id)
  }

  metaPath(id: string): string {
    return join(this.entryDir(id), 'meta.json')
  }

  secretPath(id: string): string {
    return join(this.entryDir(id), this.secretFileName)
  }

  activePath(): string {
    return join(this.root, 'active')
  }

  async list(): Promise<ProviderCredentialInfo[]> {
    const entries = await this.listMeta()
    return entries.map((m) => ({
      id: m.id,
      label: m.label,
      detail: m.detail ?? null,
      updatedAt: m.updatedAt,
    }))
  }

  async listMeta(): Promise<CredentialEntryMeta[]> {
    let names: string[]
    try {
      names = await readdir(this.entriesDir())
    } catch (error) {
      if (!isMissingEntryError(error))
        throw error
      return []
    }
    const out: CredentialEntryMeta[] = []
    for (const name of names) {
      const meta = await this.readMeta(name)
      if (meta)
        out.push(meta)
    }
    out.sort((a, b) => a.id.localeCompare(b.id))
    return out
  }

  /**
   * The entry's metadata, or null when there is no such entry. A `meta.json`
   * that exists but does not match the schema is corrupt pool state: it raises
   * `credential_invalid` rather than being read past with substituted values.
   */
  async readMeta(id: string): Promise<CredentialEntryMeta | null> {
    let text: string
    try {
      text = await readFile(this.metaPath(id), 'utf8')
    } catch (error) {
      if (!isMissingEntryError(error))
        throw error
      return null
    }
    let value: unknown
    try {
      value = JSON.parse(text)
    } catch (error) {
      throw new CredentialPoolError(
        'credential_invalid',
        `Credential "${id}" metadata is not JSON: ${errorMessage(error)}`
      )
    }
    const parsed = credentialEntryMetaSchema.safeParse(value)
    if (!parsed.success) {
      const issues = z.prettifyError(parsed.error)
      throw new CredentialPoolError(
        'credential_invalid',
        `Credential "${id}" metadata is invalid: ${issues}`
      )
    }
    return parsed.data
  }

  async getActiveId(): Promise<string | null> {
    let id: string
    try {
      id = (await readFile(this.activePath(), 'utf8')).trim()
    } catch (error) {
      if (!isFileNotFoundError(error))
        throw error
      return null
    }
    if (!id)
      return null
    // The pointer can name an entry that was removed: that is not corruption.
    return (await this.readMeta(id)) ? id : null
  }

  async setActiveId(id: string): Promise<void> {
    const meta = await this.readMeta(id)
    if (!meta)
      throw new CredentialPoolError(
        'credential_not_found',
        `Credential "${id}" not found`
      )
    try {
      await readFile(this.secretPath(id), 'utf8')
    } catch {
      throw new CredentialPoolError(
        'credential_not_found',
        `Credential "${id}" has no secret material`
      )
    }
    await this.withWriteLock(async () => {
      const tmp = this.tmpPath(this.activePath())
      await writeFile(tmp, `${id}\n`, { mode: 0o600 })
      await rename(tmp, this.activePath())
    })
  }

  async clearActive(): Promise<void> {
    await rm(this.activePath(), { force: true }).catch(() => undefined)
  }

  async writeEntry(
    meta: CredentialEntryMeta,
    secretText: string
  ): Promise<CredentialEntryMeta> {
    return this.withWriteLock(async () => {
      const dir = this.entryDir(meta.id)
      await mkdir(dir, { recursive: true, mode: 0o700 })
      const secretTmp = this.tmpPath(this.secretPath(meta.id))
      const metaTmp = this.tmpPath(this.metaPath(meta.id))
      await writeFile(secretTmp, secretText, { mode: 0o600 })
      await chmod(secretTmp, 0o600).catch(() => undefined)
      await rename(secretTmp, this.secretPath(meta.id))
      await writeFile(
        metaTmp,
        `${JSON.stringify(meta, null, 2)}\n`,
        { mode: 0o600 }
      )
      await rename(metaTmp, this.metaPath(meta.id))
      return meta
    })
  }

  async readSecretText(id: string): Promise<string> {
    return readFile(this.secretPath(id), 'utf8')
  }

  document(id: string): CredentialDocument {
    const path = this.secretPath(id)
    const read = async (): Promise<CredentialDocumentRevision | null> => {
      try {
        const text = await readFile(path, 'utf8')
        return { text, version: textVersion(text) }
      } catch (error) {
        if (!isMissingEntryError(error))
          throw error
        return null
      }
    }
    return {
      key: path,
      name: path,
      read,
      replace: (text, version) => this.withWriteLock(async () => {
        if ((await read())?.version !== version)
          return false
        const tmp = this.tmpPath(path)
        await writeFile(tmp, text, { mode: 0o600 })
        await rename(tmp, path)
        return true
      }),
    }
  }

  async remove(id: string): Promise<void> {
    await this.withWriteLock(async () => {
      const active = await this.getActiveId()
      await rm(this.entryDir(id), { recursive: true, force: true })
      if (active === id)
        await this.clearActive()
    })
  }

  private tmpPath(target: string): string {
    return `${target}.${process.pid}.${randomUUID().slice(0, 8)}.tmp`
  }

  /**
   * Serializes pool mutations across processes with a create-exclusive lock
   * file; stale locks (mtime older than 30s) are removed.
   */
  private async withWriteLock<T>(fn: () => Promise<T>): Promise<T> {
    await mkdir(this.root, { recursive: true, mode: 0o700 })
    const lockPath = join(this.root, '.lock')
    const started = Date.now()
    while (true) {
      try {
        await writeFile(
          lockPath,
          `${process.pid}\n`,
          { flag: 'wx', mode: 0o600 }
        )
        break
      } catch (error) {
        if (errorCode(error) !== 'EEXIST')
          throw error
        const info = await stat(lockPath).catch(() => null)
        if (info && Date.now() - info.mtimeMs > 30_000) {
          await rm(lockPath, { force: true }).catch(() => undefined)
          continue
        }
        if (Date.now() - started > 5_000) {
          throw new CredentialPoolError(
            'credential_invalid',
            `Timed out waiting for credential pool lock ${lockPath}`
          )
        }
        await new Promise((resolve) => setTimeout(resolve, 25))
      }
    }
    try {
      return await fn()
    } finally {
      await rm(lockPath, { force: true }).catch(() => undefined)
    }
  }

  async findByIdentityKey(
    identityKey: string
  ): Promise<CredentialEntryMeta | null> {
    const all = await this.listMeta()
    return all.find((m) => m.identityKey === identityKey) ?? null
  }

  /**
   * If active missing but entries exist, pick first and repair active pointer.
   */
  async ensureActivePointer(): Promise<string | null> {
    const active = await this.getActiveId()
    if (active)
      return active
    const all = await this.listMeta()
    if (all.length === 0)
      return null
    await this.setActiveId(all[0]!.id)
    return all[0]!.id
  }
}

/**
 * A pool held in memory: the accounts of something that is not stored yet (a
 * login before it completes), and the pool of tests.
 */
export class MemoryCredentialPool implements CredentialPool {
  private readonly held = new Map<string, {
    meta: CredentialEntryMeta
    text: string
    version: number
  }>()
  private activeId: string | null = null

  constructor(readonly vendorDefault = false) {}

  /** Every account with its secret text, for whoever stores the pool. */
  entries(): Array<{ meta: CredentialEntryMeta; secretText: string }> {
    return [...this.held.values()].map(
      (entry) => ({ meta: entry.meta, secretText: entry.text })
    )
  }

  async list(): Promise<ProviderCredentialInfo[]> {
    return (await this.listMeta()).map((m) => ({
      id: m.id,
      label: m.label,
      detail: m.detail ?? null,
      updatedAt: m.updatedAt,
    }))
  }

  async listMeta(): Promise<CredentialEntryMeta[]> {
    return [...this.held.values()]
      .map((entry) => entry.meta)
      .sort((a, b) => a.id.localeCompare(b.id))
  }

  async readMeta(id: string): Promise<CredentialEntryMeta | null> {
    return this.held.get(id)?.meta ?? null
  }

  async findByIdentityKey(
    identityKey: string
  ): Promise<CredentialEntryMeta | null> {
    return (await this.listMeta())
      .find((m) => m.identityKey === identityKey) ?? null
  }

  async getActiveId(): Promise<string | null> {
    return this.activeId
  }

  async setActiveId(id: string): Promise<void> {
    if (!this.held.has(id))
      throw new CredentialPoolError(
        'credential_not_found',
        `Credential "${id}" not found`
      )
    this.activeId = id
  }

  async ensureActivePointer(): Promise<string | null> {
    this.activeId ??= (await this.listMeta())[0]?.id ?? null
    return this.activeId
  }

  async writeEntry(
    meta: CredentialEntryMeta,
    secretText: string
  ): Promise<CredentialEntryMeta> {
    this.held.set(meta.id, {
      meta,
      text: secretText,
      version: (this.held.get(meta.id)?.version ?? 0) + 1,
    })
    return meta
  }

  document(id: string): CredentialDocument {
    return {
      key: `memory/${id}`,
      name: `account ${id}`,
      read: async () => {
        const entry = this.held.get(id)
        return entry
          ? { text: entry.text, version: String(entry.version) }
          : null
      },
      replace: async (text, version) => {
        const entry = this.held.get(id)
        if (!entry || String(entry.version) !== version)
          return false
        this.held.set(id, { ...entry, text, version: entry.version + 1 })
        return true
      },
    }
  }

  async remove(id: string): Promise<void> {
    this.held.delete(id)
    if (this.activeId === id)
      this.activeId = null
  }
}

export class CredentialPoolError extends Error {
  constructor(
    readonly code: 'credential_not_found' | 'credential_invalid',
    message: string,
  ) {
    super(message)
    this.name = 'CredentialPoolError'
  }
}

/**
 * Writes `value` as pretty-printed JSON that no reader ever sees half-written:
 * the bytes land in a sibling temporary file, readable by the owner only, and
 * the rename that publishes them is atomic. Used for the vendor `auth.json`
 * files a provider owns outside the pool.
 */
export async function writeJsonFileAtomic(
  path: string,
  value: unknown
): Promise<void> {
  await mkdir(dirname(path), { recursive: true })
  const temp = `${path}.${process.pid}.${randomUUID().slice(0, 8)}.tmp`
  await writeFile(temp, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 })
  await chmod(temp, 0o600)
  await rename(temp, path)
}

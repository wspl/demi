/**
 * Demi multi-credential pool on disk:
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

export class FileCredentialPool {
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

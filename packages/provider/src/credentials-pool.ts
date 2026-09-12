/**
 * Demi multi-credential pool on disk:
 * <stateDir>/credentials/<providerKey>/{active,entries/<id>/{meta.json,secret}}
 */
import { errorCode, nonEmptyString } from '@demicodes/utils'
import { z } from 'zod'
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
import { join, resolve } from 'node:path'
import type { ProviderCredentialInfo } from './types'

const credentialIdSchema = z.string().regex(/^[A-Za-z0-9][A-Za-z0-9_-]*$/)
const metadataTextSchema = z.string().regex(/\S/)
const credentialEntryMetaSchema = z.strictObject({
  id: credentialIdSchema,
  label: metadataTextSchema,
  detail: metadataTextSchema.nullable().optional(),
  updatedAt: z.iso.datetime({ offset: true }),
  source: metadataTextSchema.nullable().optional(),
  identityKey: metadataTextSchema.nullable().optional(),
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
    if (!credentialIdSchema.safeParse(id).success) {
      throw new CredentialPoolError('credential_invalid', 'Invalid credential ID')
    }
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
      if (errorCode(error) === 'ENOENT') {
        return []
      }
      throw error
    }
    const out: CredentialEntryMeta[] = []
    for (const name of names) {
      const meta = await this.readMeta(name)
      if (!meta) {
        throw new CredentialPoolError(
          'credential_invalid',
          `Credential "${name}" has no metadata`
        )
      }
      out.push(meta)
    }
    out.sort((a, b) => a.id.localeCompare(b.id))
    return out
  }

  async readMeta(id: string): Promise<CredentialEntryMeta | null> {
    let text: string
    try {
      text = await readFile(this.metaPath(id), 'utf8')
    } catch (error) {
      if (errorCode(error) === 'ENOENT') {
        return null
      }
      throw error
    }
    let raw: unknown
    try {
      raw = JSON.parse(text)
    } catch {
      throw new CredentialPoolError('credential_invalid', `Credential "${id}" has invalid metadata JSON`)
    }
    const result = credentialEntryMetaSchema.safeParse(raw)
    if (!result.success) {
      const fields = result.error.issues.map((issue) => issue.path.join('.') || 'metadata')
      throw new CredentialPoolError(
        'credential_invalid',
        `Credential "${id}" has invalid metadata fields: ${fields.join(', ')}`
      )
    }
    if (result.data.id !== id) {
      throw new CredentialPoolError('credential_invalid', `Credential "${id}" metadata ID does not match its directory`)
    }
    return result.data
  }

  private async readActivePointer(): Promise<string | null> {
    let text: string
    try {
      text = await readFile(this.activePath(), 'utf8')
    } catch (error) {
      if (errorCode(error) === 'ENOENT') {
        return null
      }
      throw error
    }
    // The pointer is one ID, optionally followed by the writer's newline.
    const parsed = credentialIdSchema.safeParse(text.endsWith('\n') ? text.slice(0, -1) : text)
    if (!parsed.success) {
      throw new CredentialPoolError('credential_invalid', 'Invalid active credential pointer')
    }
    return parsed.data
  }

  async getActiveId(): Promise<string | null> {
    const id = await this.readActivePointer()
    if (id === null) {
      return null
    }
    const meta = await this.readMeta(id)
    if (!meta) {
      throw new CredentialPoolError('credential_invalid', 'Active credential metadata is missing')
    }
    return meta.id
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
    } catch (error) {
      if (errorCode(error) !== 'ENOENT') {
        throw error
      }
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
    await rm(this.activePath(), { force: true })
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
      const active = await this.readActivePointer()
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
   * If the pointer is absent, select the first entry; invalid pointers fail.
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


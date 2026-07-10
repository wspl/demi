/**
 * Demi multi-credential pool on disk:
 *   <stateDir>/credentials/<providerKey>/{active,entries/<id>/{meta.json,secret}}
 */
import { errorCode, isRecord, nonEmptyString } from '@demicodes/utils'
import { createHash, randomUUID } from 'node:crypto'
import { chmod, mkdir, readdir, readFile, rename, rm, stat, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join, resolve } from 'node:path'
import type { ProviderCredentialInfo } from './types'

export interface CredentialEntryMeta {
  id: string
  label: string
  detail?: string | null
  updatedAt: string
  source?: string | null
  /** Stable account key for upsert on re-import (email, accountId, entryKey, …). */
  identityKey?: string | null
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
 * Demi local state root (`$DEMI_HOME` / `~/.demi`). Canonical copy — host-local
 * re-exports this for its bridge layout.
 */
export function resolveDemiHome(explicit?: string): string {
  if (explicit && explicit.trim()) return resolve(explicit.trim())
  const fromEnv = process.env.DEMI_HOME
  if (fromEnv && fromEnv.trim()) return resolve(fromEnv.trim())
  return join(homedir(), '.demi')
}

export function credentialIdFromIdentity(identityKey: string | null | undefined, fallbackLabel: string): string {
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
    } catch {
      return []
    }
    const out: CredentialEntryMeta[] = []
    for (const name of names) {
      const meta = await this.readMeta(name)
      if (meta) out.push(meta)
    }
    out.sort((a, b) => a.id.localeCompare(b.id))
    return out
  }

  async readMeta(id: string): Promise<CredentialEntryMeta | null> {
    try {
      const raw = JSON.parse(await readFile(this.metaPath(id), 'utf8')) as unknown
      if (!isRecord(raw)) return null
      const entryId = nonEmptyString(raw.id) ?? id
      const label = nonEmptyString(raw.label)
      if (!label) return null
      return {
        id: entryId,
        label,
        detail: nonEmptyString(raw.detail) ?? null,
        updatedAt: nonEmptyString(raw.updatedAt) ?? new Date(0).toISOString(),
        source: nonEmptyString(raw.source) ?? null,
        identityKey: nonEmptyString(raw.identityKey) ?? null,
      }
    } catch {
      return null
    }
  }

  async getActiveId(): Promise<string | null> {
    try {
      const id = (await readFile(this.activePath(), 'utf8')).trim()
      if (!id) return null
      const meta = await this.readMeta(id)
      return meta ? id : null
    } catch {
      return null
    }
  }

  async setActiveId(id: string): Promise<void> {
    const meta = await this.readMeta(id)
    if (!meta) throw new CredentialPoolError('credential_not_found', `Credential "${id}" not found`)
    try {
      await readFile(this.secretPath(id), 'utf8')
    } catch {
      throw new CredentialPoolError('credential_not_found', `Credential "${id}" has no secret material`)
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

  async writeEntry(meta: CredentialEntryMeta, secretText: string): Promise<CredentialEntryMeta> {
    return this.withWriteLock(async () => {
      const dir = this.entryDir(meta.id)
      await mkdir(dir, { recursive: true, mode: 0o700 })
      const secretTmp = this.tmpPath(this.secretPath(meta.id))
      const metaTmp = this.tmpPath(this.metaPath(meta.id))
      await writeFile(secretTmp, secretText, { mode: 0o600 })
      await chmod(secretTmp, 0o600).catch(() => undefined)
      await rename(secretTmp, this.secretPath(meta.id))
      await writeFile(metaTmp, `${JSON.stringify(meta, null, 2)}\n`, { mode: 0o600 })
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
      if (active === id) await this.clearActive()
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
        await writeFile(lockPath, `${process.pid}\n`, { flag: 'wx', mode: 0o600 })
        break
      } catch (error) {
        if (errorCode(error) !== 'EEXIST') throw error
        const info = await stat(lockPath).catch(() => null)
        if (info && Date.now() - info.mtimeMs > 30_000) {
          await rm(lockPath, { force: true }).catch(() => undefined)
          continue
        }
        if (Date.now() - started > 5_000) {
          throw new CredentialPoolError('credential_invalid', `Timed out waiting for credential pool lock ${lockPath}`)
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

  async findByIdentityKey(identityKey: string): Promise<CredentialEntryMeta | null> {
    const all = await this.listMeta()
    return all.find((m) => m.identityKey === identityKey) ?? null
  }

  /** If active missing but entries exist, pick first and repair active pointer. */
  async ensureActivePointer(): Promise<string | null> {
    const active = await this.getActiveId()
    if (active) return active
    const all = await this.listMeta()
    if (all.length === 0) return null
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

export async function runVendorLoginCommand(
  command: string,
  args: string[],
  options: { signal?: AbortSignal } = {},
): Promise<{ status: 'completed' | 'cancelled' | 'failed' | 'unavailable'; message?: string }> {
  const { spawn } = await import('node:child_process')
  return new Promise((resolvePromise) => {
    let child: ReturnType<typeof spawn>
    try {
      child = spawn(command, args, {
        stdio: 'inherit',
        env: process.env,
        shell: false,
      })
    } catch (error) {
      resolvePromise({
        status: 'unavailable',
        message: error instanceof Error ? error.message : String(error),
      })
      return
    }

    const onAbort = () => {
      child.kill('SIGTERM')
    }
    if (options.signal) {
      if (options.signal.aborted) {
        child.kill('SIGTERM')
        resolvePromise({ status: 'cancelled' })
        return
      }
      options.signal.addEventListener('abort', onAbort, { once: true })
    }

    child.on('error', (error) => {
      options.signal?.removeEventListener('abort', onAbort)
      const msg = error instanceof Error ? error.message : String(error)
      if (errorCode(error) === 'ENOENT') {
        resolvePromise({ status: 'unavailable', message: `Command not found: ${command}` })
        return
      }
      resolvePromise({ status: 'failed', message: msg })
    })

    child.on('exit', (code, signal) => {
      options.signal?.removeEventListener('abort', onAbort)
      if (options.signal?.aborted || signal === 'SIGTERM' || signal === 'SIGINT') {
        resolvePromise({ status: 'cancelled' })
        return
      }
      if (code === 0) resolvePromise({ status: 'completed' })
      else resolvePromise({ status: 'failed', message: `${command} exited with code ${code ?? 'null'}` })
    })
  })
}


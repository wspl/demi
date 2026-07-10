import type {
  ProviderCredentialActive,
  ProviderCredentialAddInput,
  ProviderCredentialInfo,
  ProviderCredentialLoginOptions,
  ProviderCredentialLoginResult,
  ProviderCredentials,
  ProviderCredentialsCapability,
  ProviderQuota,
} from '@demicodes/provider'
import { isRecord, nonEmptyString } from '@demicodes/utils'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import {
  FileGrokAuthStore,
  GrokAuthError,
  defaultGrokHome,
  selectAuthEntry,
  type FileGrokAuthStoreOptions,
  type GrokAuthDotJson,
  type GrokAuthEntry,
  type GrokAuthStore,
} from './auth'
import {
  FileCredentialPool,
  credentialIdFromIdentity,
  runVendorLoginCommand,
  type CredentialEntryMeta,
} from '@demicodes/provider/credentials-pool'

export class PoolAwareGrokAuthStore implements GrokAuthStore {
  private readonly pool: FileCredentialPool
  private readonly vendorHome: string
  private readonly fileAuthOptions: Omit<FileGrokAuthStoreOptions, 'grokHome' | 'authFile' | 'entryKey'>

  constructor(
    pool: FileCredentialPool,
    options: { grokHome?: string; fileAuthOptions?: Omit<FileGrokAuthStoreOptions, 'grokHome' | 'authFile' | 'entryKey'> } = {},
  ) {
    this.pool = pool
    this.vendorHome = options.grokHome ?? defaultGrokHome()
    this.fileAuthOptions = options.fileAuthOptions ?? {}
  }

  async status() {
    return this.currentStore().then((s) => s.status())
  }

  async resolveAuth(options?: { forceRefresh?: boolean }) {
    return this.currentStore().then((s) => s.resolveAuth(options))
  }

  private async currentStore(): Promise<FileGrokAuthStore> {
    await this.pool.ensureActivePointer()
    const activeId = await this.pool.getActiveId()
    if (activeId) {
      const meta = await this.pool.readMeta(activeId)
      const entryKey = meta?.identityKey ?? undefined
      return new FileGrokAuthStore({
        ...this.fileAuthOptions,
        authFile: this.pool.secretPath(activeId),
        grokHome: this.pool.entryDir(activeId),
        entryKey,
      })
    }
    return new FileGrokAuthStore({
      ...this.fileAuthOptions,
      grokHome: this.vendorHome,
    })
  }
}

export function openGrokCredentialPool(options: { stateDir?: string } = {}): FileCredentialPool {
  return new FileCredentialPool({
    stateDir: options.stateDir,
    providerKey: 'grok-build',
    secretFileName: 'auth.json',
  })
}

export function createGrokBuildCredentials(
  pool: FileCredentialPool,
  authStore: GrokAuthStore,
  options: {
    grokHome?: string
    loginCommand?: string
    loginArgs?: string[]
    quota?: ProviderQuota | null
  } = {},
): ProviderCredentials {
  const vendorHome = options.grokHome ?? defaultGrokHome()
  const loginCommand = options.loginCommand ?? 'grok'
  const loginArgs = options.loginArgs ?? ['login']

  const capability = (): ProviderCredentialsCapability => ({
    mode: 'supported',
    canBeginLogin: true,
    canImportDefault: true,
    canAdd: true,
    multi: true,
  })

  const getActive = async (): Promise<ProviderCredentialActive> => {
    await pool.ensureActivePointer()
    const credentialId = await pool.getActiveId()
    const status = await authStore.status()
    return { credentialId, status }
  }

  const setActive = async (credentialId: string): Promise<ProviderCredentialActive> => {
    await pool.setActiveId(credentialId)
    options.quota?.clearLatest?.()
    return getActive()
  }

  const importEntry = async (
    entryKey: string,
    entry: GrokAuthEntry,
    source: string,
  ): Promise<ProviderCredentialInfo> => {
    const email = nonEmptyString(entry.email)
    const label = email ?? entryKey
    const identityKey = entryKey
    const existing = await pool.findByIdentityKey(identityKey)
    const id = existing?.id ?? credentialIdFromIdentity(identityKey, label)
    const file: GrokAuthDotJson = { [entryKey]: entry }
    const meta: CredentialEntryMeta = {
      id,
      label,
      detail: nonEmptyString(entry.auth_mode) ?? 'oidc',
      updatedAt: new Date().toISOString(),
      source,
      identityKey,
    }
    await pool.writeEntry(meta, `${JSON.stringify(file, null, 2)}\n`)
    const active = await pool.getActiveId()
    if (!active) await pool.setActiveId(id)
    options.quota?.clearLatest?.()
    return { id: meta.id, label: meta.label, detail: meta.detail, updatedAt: meta.updatedAt }
  }

  const importFromAuthJsonText = async (text: string, source: string): Promise<ProviderCredentialInfo[]> => {
    let file: GrokAuthDotJson
    try {
      file = JSON.parse(text) as GrokAuthDotJson
    } catch {
      throw new GrokAuthError('auth_invalid', 'Grok auth material is not valid JSON')
    }
    if (!isRecord(file)) throw new GrokAuthError('auth_invalid', 'Grok auth material is not an object')

    const imported: ProviderCredentialInfo[] = []
    for (const [entryKey, value] of Object.entries(file)) {
      if (!isRecord(value)) continue
      const entry = value as GrokAuthEntry
      if (!nonEmptyString(entry.key)) continue
      imported.push(await importEntry(entryKey, entry, source))
    }
    if (imported.length === 0) {
      throw new GrokAuthError('auth_missing', 'No Grok OAuth entries with access tokens found to import')
    }
    return imported
  }

  return {
    capability,
    list: () => pool.list(),
    getActive,
    setActive,
    beginLogin: async (loginOptions?: ProviderCredentialLoginOptions): Promise<ProviderCredentialLoginResult> => {
      const result = await runVendorLoginCommand(loginCommand, loginArgs, { signal: loginOptions?.signal })
      if (result.status === 'completed') return { status: 'completed' }
      if (result.status === 'cancelled') return { status: 'cancelled' }
      if (result.status === 'unavailable') return { status: 'unavailable', message: result.message ?? 'Login unavailable' }
      return { status: 'failed', message: result.message ?? 'Login failed' }
    },
    importDefault: async () => {
      const authFile = join(vendorHome, 'auth.json')
      let text: string
      try {
        text = await readFile(authFile, 'utf8')
      } catch {
        throw new GrokAuthError('auth_missing', `No Grok auth at ${authFile}. Run grok login or beginLogin first.`)
      }
      // Import all entries, then activate the vendor-preferred one. Entries are
      // upserted by identityKey (= map entry key), so the preferred entry is
      // found deterministically by that key — no label/detail guessing.
      const all = await importFromAuthJsonText(text, `vendor:${authFile}`)
      const preferred = selectAuthEntry(JSON.parse(text) as GrokAuthDotJson)
      if (preferred) {
        const byKey = (await pool.listMeta()).find((m) => m.identityKey === preferred.entryKey)
        if (byKey) {
          await pool.setActiveId(byKey.id)
          options.quota?.clearLatest?.()
          return { id: byKey.id, label: byKey.label, detail: byKey.detail, updatedAt: byKey.updatedAt }
        }
      }
      return all[0]!
    },
    add: async (input: ProviderCredentialAddInput) => {
      if (typeof input.authJsonText === 'string') {
        const all = await importFromAuthJsonText(input.authJsonText, 'add:authJsonText')
        return all[0]!
      }
      if (typeof input.authFile === 'string') {
        const text = await readFile(input.authFile, 'utf8')
        const all = await importFromAuthJsonText(text, `add:authFile:${input.authFile}`)
        return all[0]!
      }
      if (typeof input.entryKey === 'string' && isRecord(input.entry)) {
        return importEntry(input.entryKey, input.entry as GrokAuthEntry, 'add:entry')
      }
      throw new Error('Grok credentials.add expects authJsonText, authFile, or { entryKey, entry }')
    },
    remove: async (credentialId: string) => {
      await pool.remove(credentialId)
      options.quota?.clearLatest?.()
    },
  }
}

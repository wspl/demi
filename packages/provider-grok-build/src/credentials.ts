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
import { errorCode, errorMessage } from '@demicodes/utils'
import { parseProviderData } from '@demicodes/provider'
import { grokCredentialAddSchema, parseGrokAuthData } from './auth-schemas'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import {
  FileGrokAuthStore,
  GrokAuthError,
  defaultGrokHome,
  selectAuthEntry,
  parseGrokAuthJson,
  type FileGrokAuthStoreOptions,
  type GrokAuthDotJson,
  type GrokAuthEntry,
  type GrokAuthStore,
} from './auth'
import { runGrokDeviceLogin } from './device-login'
import {
  FileCredentialPool,
  credentialIdFromIdentity,
  type CredentialEntryMeta,
} from '@demicodes/provider/credentials-pool'

export class PoolAwareGrokAuthStore implements GrokAuthStore {
  private readonly pool: FileCredentialPool
  private readonly vendorHome: string
  private readonly fileAuthOptions: Omit<FileGrokAuthStoreOptions, 'grokHome'
    | 'authFile'
    | 'entryKey'>

  constructor(
    pool: FileCredentialPool,
    options: {
      grokHome?: string;
      fileAuthOptions?: Omit<FileGrokAuthStoreOptions, 'grokHome'
        | 'authFile'
        | 'entryKey'>
    } = {},
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
      const entryKey = meta?.identityKey
      if (!entryKey) {
        throw new GrokAuthError('auth_invalid', 'Active Grok credential has no entry identity')
      }
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

export function openGrokCredentialPool(
  options: { stateDir?: string } = {}
): FileCredentialPool {
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
    quota?: ProviderQuota | null
    clientVersion?: string
    /** Injectable fetch for the device-code login flow (tests). */
    loginFetch?: typeof fetch
  } = {},
): ProviderCredentials {
  const vendorHome = options.grokHome ?? defaultGrokHome()

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

  const setActive = async (
    credentialId: string
  ): Promise<ProviderCredentialActive> => {
    await pool.setActiveId(credentialId)
    options.quota?.clearLatest?.()
    return getActive()
  }

  const importEntry = async (
    entryKey: string,
    entry: GrokAuthEntry,
    source: string,
  ): Promise<ProviderCredentialInfo> => {
    const email = entry.email
    const label = email ?? entryKey
    const identityKey = entryKey
    const existing = await pool.findByIdentityKey(identityKey)
    const id = existing?.id ?? credentialIdFromIdentity(identityKey, label)
    const file: GrokAuthDotJson = { [entryKey]: entry }
    const meta: CredentialEntryMeta = {
      id,
      label,
      detail: entry.auth_mode ?? 'oidc',
      updatedAt: new Date().toISOString(),
      source,
      identityKey,
    }
    await pool.writeEntry(meta, `${JSON.stringify(file, null, 2)}\n`)
    const active = await pool.getActiveId()
    if (!active)
      await pool.setActiveId(id)
    options.quota?.clearLatest?.()
    return {
      id: meta.id,
      label: meta.label,
      detail: meta.detail,
      updatedAt: meta.updatedAt
    }
  }

  const importAuthFile = async (
    file: GrokAuthDotJson,
    source: string,
  ): Promise<ProviderCredentialInfo> => {
    const preferred = selectAuthEntry(file)
    if (!preferred) {
      throw new GrokAuthError('auth_missing', 'No Grok credentials found to import')
    }
    // Select deterministically from the same validated map used for all writes.
    const preferredInfo = await importEntry(preferred.entryKey, preferred.entry, source)
    for (const [entryKey, entry] of Object.entries(file)) {
      if (entryKey !== preferred.entryKey) {
        await importEntry(entryKey, entry, source)
      }
    }
    return preferredInfo
  }

  return {
    capability,
    list: () => pool.list(),
    getActive,
    setActive,
    // Native RFC 8628 device flow: pending material streams out via onPending, the completed
    // entry never touches the vendor home and is imported straight into the pool.
    beginLogin: async (
      loginOptions?: ProviderCredentialLoginOptions
    ): Promise<ProviderCredentialLoginResult> => {
      try {
        const { entryKey, entry } = await runGrokDeviceLogin({
          signal: loginOptions?.signal,
          onPending: loginOptions?.onPending,
          fetch: options.loginFetch,
          surface: 'ui',
          grokHome: vendorHome,
          clientVersion: options.clientVersion,
        })
        const info = await importAuthFile(parseGrokAuthData({ [entryKey]: entry }), 'login:device')
        return { status: 'completed', credentialId: info.id }
      } catch (error) {
        if (loginOptions?.signal?.aborted)
          return { status: 'cancelled' }
        return { status: 'failed', message: errorMessage(error) }
      }
    },
    importDefault: async () => {
      const authFile = join(vendorHome, 'auth.json')
      let text: string
      try {
        text = await readFile(authFile, 'utf8')
      } catch (error) {
        if (errorCode(error) !== 'ENOENT') {
          throw error
        }
        throw new GrokAuthError(
          'auth_missing',
          `No Grok auth at ${authFile}. Run grok login or beginLogin first.`
        )
      }
      const info = await importAuthFile(parseGrokAuthJson(text), `vendor:${authFile}`)
      await pool.setActiveId(info.id)
      options.quota?.clearLatest?.()
      return info
    },
    add: async (input: ProviderCredentialAddInput) => {
      const value = parseProviderData(grokCredentialAddSchema, input, 'Grok credential input')
      if ('authJsonText' in value) {
        return importAuthFile(parseGrokAuthJson(value.authJsonText), 'add:authJsonText')
      }
      if ('authFile' in value) {
        const text = await readFile(value.authFile, 'utf8')
        return importAuthFile(parseGrokAuthJson(text), `add:authFile:${value.authFile}`)
      }
      return importAuthFile(parseGrokAuthData({ [value.entryKey]: value.entry }), 'add:entry')
    },
    remove: async (credentialId: string) => {
      await pool.remove(credentialId)
      options.quota?.clearLatest?.()
    },
  }
}

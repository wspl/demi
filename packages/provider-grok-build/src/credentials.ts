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
import { errorMessage, nonEmptyString } from '@demicodes/utils'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import {
  FileGrokAuthStore,
  GrokAuthError,
  defaultGrokHome,
  grokAuthEntrySchema,
  parseGrokAuthDotJson,
  selectAuthEntry,
  type FileGrokAuthStoreOptions,
  type GrokAuthDotJson,
  type GrokAuthEntry,
  type GrokAuthStore,
} from './auth'
import { runGrokDeviceLogin } from './device-login'
import {
  FileCredentialPool,
  type CredentialPool,
  credentialIdFromIdentity,
  type CredentialEntryMeta,
} from '@demicodes/provider/credentials-pool'

/**
 * Auth store that prefers the demi pool active entry; falls back to vendor
 * ~/.grok.
 */
export class PoolAwareGrokAuthStore implements GrokAuthStore {
  private readonly pool: CredentialPool
  private readonly vendorHome: string
  private readonly credentialId: string | null
  private readonly fileAuthOptions: Omit<FileGrokAuthStoreOptions, 'grokHome'
    | 'authFile'
    | 'document'
    | 'entryKey'>

  constructor(
    pool: CredentialPool,
    options: {
      grokHome?: string;
      /** The account this store stands for, instead of the pool's active one. */
      credentialId?: string;
      fileAuthOptions?: Omit<FileGrokAuthStoreOptions, 'grokHome'
        | 'authFile'
        | 'document'
        | 'entryKey'>
    } = {},
  ) {
    this.pool = pool
    this.vendorHome = options.grokHome ?? defaultGrokHome()
    this.credentialId = options.credentialId ?? null
    this.fileAuthOptions = options.fileAuthOptions ?? {}
  }

  async status() {
    return this.currentStore().then((s) => s.status())
  }

  async resolveAuth(options?: { forceRefresh?: boolean }) {
    return this.currentStore().then((s) => s.resolveAuth(options))
  }

  private async currentStore(): Promise<GrokAuthStore> {
    const id = this.credentialId ?? await this.pool.ensureActivePointer()
    if (id) {
      const meta = await this.pool.readMeta(id)
      return new FileGrokAuthStore({
        ...this.fileAuthOptions,
        document: this.pool.document(id),
        entryKey: meta?.identityKey ?? undefined,
      })
    }
    if (!this.pool.vendorDefault)
      return new MissingGrokAuthStore()
    return new FileGrokAuthStore({
      ...this.fileAuthOptions,
      grokHome: this.vendorHome,
    })
  }
}

/** An entry without an account, where no vendor login may stand in for one. */
class MissingGrokAuthStore implements GrokAuthStore {
  async status() {
    return {
      status: 'unauthenticated' as const,
      message: 'No Grok account is signed in'
    }
  }

  async resolveAuth(): Promise<never> {
    throw new GrokAuthError('auth_missing', 'No Grok account is signed in')
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
  pool: CredentialPool,
  authStore: GrokAuthStore,
  options: {
    grokHome?: string
    quota?: ProviderQuota | null
    /**
     * The provider stands for one named account: another account becoming
     * active or being added says nothing about its usage.
     */
    pinned?: boolean
    /** Injectable fetch for the device-code login flow (tests). */
    loginFetch?: typeof fetch
  } = {},
): ProviderCredentials {
  const vendorHome = options.grokHome ?? defaultGrokHome()

  const capability = (): ProviderCredentialsCapability => ({
    mode: 'supported',
    canBeginLogin: true,
    canImportDefault: pool.vendorDefault,
    canAdd: true,
    multi: true,
  })

  const accountChanged = (): void => {
    if (!options.pinned)
      options.quota?.clearLatest?.()
  }

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
    accountChanged()
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
    if (!active)
      await pool.setActiveId(id)
    accountChanged()
    return {
      id: meta.id,
      label: meta.label,
      detail: meta.detail,
      updatedAt: meta.updatedAt
    }
  }

  /** Every entry of an auth.json that carries an access token. */
  const importEntries = async (
    file: GrokAuthDotJson,
    source: string
  ): Promise<ProviderCredentialInfo[]> => {
    const imported: ProviderCredentialInfo[] = []
    for (const [entryKey, entry] of Object.entries(file)) {
      if (!nonEmptyString(entry.key))
        continue
      imported.push(await importEntry(entryKey, entry, source))
    }
    if (imported.length === 0) {
      throw new GrokAuthError(
        'auth_missing',
        'No Grok OAuth entries with access tokens found to import'
      )
    }
    return imported
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
        })
        const info = await importEntry(entryKey, entry, 'login:device')
        return { status: 'completed', credentialId: info.id }
      } catch (error) {
        if (loginOptions?.signal?.aborted)
          return { status: 'cancelled' }
        return { status: 'failed', message: errorMessage(error) }
      }
    },
    importDefault: async () => {
      if (!pool.vendorDefault)
        throw new GrokAuthError(
          'auth_unsupported',
          'This pool does not take the vendor login of the machine'
        )
      const authFile = join(vendorHome, 'auth.json')
      let text: string
      try {
        text = await readFile(authFile, 'utf8')
      } catch {
        throw new GrokAuthError(
          'auth_missing',
          `No Grok auth at ${authFile}. Run grok login or beginLogin first.`
        )
      }
      // Import all entries, then activate the vendor-preferred one. Entries are
      // upserted by identityKey (= map entry key), so the preferred entry is
      // found deterministically by that key — no label/detail guessing.
      const file = parseGrokAuthDotJson(text, `Grok auth file ${authFile}`)
      const all = await importEntries(file, `vendor:${authFile}`)
      const preferred = selectAuthEntry(file)
      if (preferred) {
        const byKey = (await pool.listMeta()).find((m) => m.identityKey === preferred.entryKey)
        if (byKey) {
          await pool.setActiveId(byKey.id)
          accountChanged()
          return {
            id: byKey.id,
            label: byKey.label,
            detail: byKey.detail,
            updatedAt: byKey.updatedAt
          }
        }
      }
      return all[0]!
    },
    add: async (input: ProviderCredentialAddInput) => {
      if (typeof input.authJsonText === 'string') {
        const file = parseGrokAuthDotJson(
          input.authJsonText,
          'Grok auth material'
        )
        const all = await importEntries(file, 'add:authJsonText')
        return all[0]!
      }
      if (typeof input.authFile === 'string') {
        const text = await readFile(input.authFile, 'utf8')
        const file = parseGrokAuthDotJson(
          text,
          `Grok auth file ${input.authFile}`
        )
        const all = await importEntries(file, `add:authFile:${input.authFile}`)
        return all[0]!
      }
      if (typeof input.entryKey === 'string' && input.entry !== undefined) {
        return importEntry(
          input.entryKey,
          grokAuthEntrySchema.parse(input.entry),
          'add:entry'
        )
      }
      throw new Error(
        'Grok credentials.add expects authJsonText, authFile, or { entryKey, entry }'
      )
    },
    remove: async (credentialId: string) => {
      await pool.remove(credentialId)
      accountChanged()
    },
  }
}

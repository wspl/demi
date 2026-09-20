import type {
  ProviderAuthState,
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
import {
  GrokAuthError,
  redactGrokSecretText,
  GrokDocumentAuthStore,
  grokAuthEntrySchema,
  parseGrokAuthDotJson,
  type GrokAuthDotJson,
  type GrokAuthEntry,
  type GrokAuthStore,
  type GrokTokenRefresh,
} from './auth'
import { labelFromGrokEntry } from './vendor'
import { runGrokDeviceLogin } from './device-login'
import {
  FileCredentialPool,
  type CredentialPool,
  credentialIdFromIdentity,
  type CredentialEntryMeta,
} from '@demicodes/provider/credentials-pool'

/**
 * The auth store of a pool: the account it was built for, or the one the pool
 * has active. Which keeper the pool is makes no difference here.
 */
export class PoolAwareGrokAuthStore implements GrokAuthStore {
  constructor(
    private readonly pool: CredentialPool,
    private readonly options: {
      /** The account this store stands for, instead of the pool's active one. */
      credentialId?: string;
      refresh?: GrokTokenRefresh;
      now?: () => Date
    } = {},
  ) {}

  async status(): Promise<ProviderAuthState> {
    try {
      return await (await this.currentStore()).status()
    } catch (error) {
      // A pool that cannot be read is a state to report, like a store's own.
      return { status: 'error', message: redactGrokSecretText(errorMessage(error)) }
    }
  }

  async resolveAuth(options?: { forceRefresh?: boolean }) {
    return this.currentStore().then((s) => s.resolveAuth(options))
  }

  private async currentStore(): Promise<GrokAuthStore> {
    const id = this.options.credentialId
      ?? await this.pool.ensureActivePointer()
    if (!id)
      return new MissingGrokAuthStore()
    // An account's identity key is the key of its entry in the auth map.
    const meta = await this.pool.readMeta(id)
    return new GrokDocumentAuthStore({
      document: this.pool.document(id),
      entryKey: meta?.identityKey ?? undefined,
      refresh: this.options.refresh,
      now: this.options.now,
    })
  }
}

/** A pool without an account. */
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
    /** The vendor logins `importDefault` copies from; none, no import. */
    importFrom?: CredentialPool
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
  const capability = (): ProviderCredentialsCapability => ({
    mode: 'supported',
    canBeginLogin: true,
    canImportDefault: options.importFrom !== undefined,
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
    const { label, identityKey, detail } = labelFromGrokEntry(entryKey, entry)
    const existing = await pool.findByIdentityKey(identityKey)
    const id = existing?.id ?? credentialIdFromIdentity(identityKey, label)
    const file: GrokAuthDotJson = { [entryKey]: entry }
    const meta: CredentialEntryMeta = {
      id,
      label,
      detail,
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
    ...(options.importFrom ? {
      // Every vendor login becomes an account, and the one the vendor has in
      // use becomes the active one.
      importDefault: async () => {
        const from = options.importFrom!
        const imported = new Map<string, ProviderCredentialInfo>()
        for (const account of await from.listMeta()) {
          const revision = await from.document(account.id).read()
          const entryKey = account.identityKey
          const entry = revision && entryKey
            ? parseGrokAuthDotJson(revision.text, 'Grok auth material')[entryKey]
            : undefined
          if (entryKey && entry && nonEmptyString(entry.key)) {
            imported.set(
              account.id,
              await importEntry(entryKey, entry, account.source ?? 'vendor')
            )
          }
        }
        const activeId = await from.ensureActivePointer()
        const active = activeId ? imported.get(activeId) : undefined
        if (!active)
          throw new GrokAuthError(
            'auth_missing',
            'No Grok login on this machine. Run grok login or beginLogin first.'
          )
        await pool.setActiveId(active.id)
        accountChanged()
        return active
      },
    } : {}),
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

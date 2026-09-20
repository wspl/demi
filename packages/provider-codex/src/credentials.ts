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
import { errorMessage, isRecord } from '@demicodes/utils'
import { readFile } from 'node:fs/promises'
import {
  CodexAuthError,
  redactCodexSecretText,
  CodexDocumentAuthStore,
  parseCodexAuthDotJson,
  type CodexAuthStore,
  type CodexTokenRefresh,
} from './auth'
import { labelFromCodexAuth } from './vendor'
import { runCodexDeviceLogin } from './device-login'
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
export class PoolAwareCodexAuthStore implements CodexAuthStore {
  constructor(
    private readonly pool: CredentialPool,
    private readonly options: {
      /** The account this store stands for, instead of the pool's active one. */
      credentialId?: string;
      refresh?: CodexTokenRefresh;
      now?: () => Date
    } = {},
  ) {}

  async status(): Promise<ProviderAuthState> {
    try {
      return await (await this.currentStore()).status()
    } catch (error) {
      // A pool that cannot be read is a state to report, like a store's own.
      return { status: 'error', message: redactCodexSecretText(errorMessage(error)) }
    }
  }

  async resolveAuth(options?: { forceRefresh?: boolean }) {
    return this.currentStore().then((s) => s.resolveAuth(options))
  }

  private async currentStore(): Promise<CodexAuthStore> {
    const id = this.options.credentialId
      ?? await this.pool.ensureActivePointer()
    return id
      ? new CodexDocumentAuthStore({
        document: this.pool.document(id),
        refresh: this.options.refresh,
        now: this.options.now,
      })
      : new MissingCodexAuthStore()
  }
}

/** A pool without an account. */
class MissingCodexAuthStore implements CodexAuthStore {
  async status() {
    return {
      status: 'unauthenticated' as const,
      message: 'No Codex account is signed in'
    }
  }

  async resolveAuth(): Promise<never> {
    throw new CodexAuthError('auth_missing', 'No Codex account is signed in')
  }
}

export function createCodexCredentials(
  pool: CredentialPool,
  authStore: CodexAuthStore,
  options: {
    /** The vendor login `importDefault` copies from; none, no import. */
    importFrom?: CredentialPool
    quota?: ProviderQuota | null
    /**
     * The provider stands for one named account: another account becoming
     * active or being added says nothing about its usage.
     */
    pinned?: boolean
    onActiveChange?: () => void
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
    options.onActiveChange?.()
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

  const importFromAuthJson = async (
    authText: string,
    source: string
  ): Promise<ProviderCredentialInfo> => {
    const auth = parseCodexAuthDotJson(authText, 'Codex auth material')
    const { label, identityKey, detail } = labelFromCodexAuth(auth)
    const existing = identityKey
      ? await pool.findByIdentityKey(identityKey)
      : null
    const id = existing?.id ?? credentialIdFromIdentity(identityKey, label)
    const meta: CredentialEntryMeta = {
      id,
      label,
      detail,
      updatedAt: new Date().toISOString(),
      source,
      identityKey,
    }
    await pool.writeEntry(meta, `${JSON.stringify(auth, null, 2)}\n`)
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

  return {
    capability,
    list: () => pool.list(),
    getActive,
    setActive,
    // Native device-code flow: pending material streams out via onPending, the completed
    // material never touches the vendor home and is imported straight into the pool.
    beginLogin: async (
      loginOptions?: ProviderCredentialLoginOptions
    ): Promise<ProviderCredentialLoginResult> => {
      try {
        const auth = await runCodexDeviceLogin({
          signal: loginOptions?.signal,
          onPending: loginOptions?.onPending,
          fetch: options.loginFetch,
        })
        const info = await importFromAuthJson(
          `${JSON.stringify(auth, null, 2)}\n`,
          'login:device'
        )
        return { status: 'completed', credentialId: info.id }
      } catch (error) {
        if (loginOptions?.signal?.aborted)
          return { status: 'cancelled' }
        if (error instanceof CodexAuthError && error.code
          === 'auth_unsupported') {
          return { status: 'unavailable', message: error.message }
        }
        return { status: 'failed', message: errorMessage(error) }
      }
    },
    ...(options.importFrom ? {
      importDefault: async () => {
        const from = options.importFrom!
        const id = await from.ensureActivePointer()
        const revision = id ? await from.document(id).read() : null
        if (!id || !revision)
          throw new CodexAuthError(
            'auth_missing',
            'No Codex login on this machine. Run codex login or beginLogin first.'
          )
        const source = (await from.readMeta(id))?.source ?? 'vendor'
        return importFromAuthJson(revision.text, source)
      },
    } : {}),
    add: async (input: ProviderCredentialAddInput) => {
      if (typeof input.authJsonText === 'string') {
        return importFromAuthJson(input.authJsonText, 'add:authJsonText')
      }
      if (typeof input.authFile === 'string') {
        const text = await readFile(input.authFile, 'utf8')
        return importFromAuthJson(text, `add:authFile:${input.authFile}`)
      }
      const authObject = input.auth ?? input.authJson
      if (isRecord(authObject)) {
        return importFromAuthJson(
          `${JSON.stringify(authObject, null, 2)}\n`,
          'add:auth'
        )
      }
      throw new Error(
        'Codex credentials.add expects authJsonText, authFile, or auth/authJson object'
      )
    },
    remove: async (credentialId: string) => {
      await pool.remove(credentialId)
      accountChanged()
    },
  }
}

export function openCodexCredentialPool(
  options: { stateDir?: string } = {}
): FileCredentialPool {
  return new FileCredentialPool({
    stateDir: options.stateDir,
    providerKey: 'codex',
    secretFileName: 'auth.json',
  })
}

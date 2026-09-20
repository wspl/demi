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
import { errorMessage, isRecord, nonEmptyString } from '@demicodes/utils'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import {
  CodexAuthError,
  FileCodexAuthStore,
  defaultCodexHome,
  parseChatGptClaims,
  parseCodexAuthDotJson,
  parseIdTokenClaims,
  type CodexAuthDotJson,
  type CodexAuthStore,
  type FileCodexAuthStoreOptions,
} from './auth'
import { runCodexDeviceLogin } from './device-login'
import {
  FileCredentialPool,
  type CredentialPool,
  credentialIdFromIdentity,
  type CredentialEntryMeta,
} from '@demicodes/provider/credentials-pool'

export interface CodexCredentialsOptions {
  stateDir?: string
  codexHome?: string
  /** Shared with createCodexProvider for auth resolution. */
  fileAuthOptions?: Omit<FileCodexAuthStoreOptions, 'codexHome' | 'authFile'>
  onActiveChange?: () => void
}

/**
 * Auth store that prefers the demi pool active entry; falls back to vendor
 * ~/.codex.
 */
export class PoolAwareCodexAuthStore implements CodexAuthStore {
  private readonly pool: CredentialPool
  private readonly vendorHome: string
  private readonly credentialId: string | null
  private readonly fileAuthOptions: Omit<FileCodexAuthStoreOptions, 'codexHome'
    | 'authFile'
    | 'document'>

  constructor(
    pool: CredentialPool,
    options: {
      codexHome?: string;
      /** The account this store stands for, instead of the pool's active one. */
      credentialId?: string;
      fileAuthOptions?: Omit<FileCodexAuthStoreOptions, 'codexHome' | 'authFile' | 'document'>
    } = {},
  ) {
    this.pool = pool
    this.vendorHome = options.codexHome ?? defaultCodexHome()
    this.credentialId = options.credentialId ?? null
    this.fileAuthOptions = options.fileAuthOptions ?? {}
  }

  async status() {
    return this.currentStore().then((s) => s.status())
  }

  async resolveAuth(options?: { forceRefresh?: boolean }) {
    return this.currentStore().then((s) => s.resolveAuth(options))
  }

  private async currentStore(): Promise<CodexAuthStore> {
    const id = this.credentialId ?? await this.pool.ensureActivePointer()
    if (id) {
      return new FileCodexAuthStore({
        ...this.fileAuthOptions,
        document: this.pool.document(id),
      })
    }
    if (!this.pool.vendorDefault)
      return new MissingCodexAuthStore()
    return new FileCodexAuthStore({
      ...this.fileAuthOptions,
      codexHome: this.vendorHome,
    })
  }
}

/** An entry without an account, where no vendor login may stand in for one. */
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
    codexHome?: string
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
  const vendorHome = options.codexHome ?? defaultCodexHome()

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
    importDefault: async () => {
      if (!pool.vendorDefault)
        throw new CodexAuthError(
          'auth_unsupported',
          'This pool does not take the vendor login of the machine'
        )
      const authFile = join(vendorHome, 'auth.json')
      let text: string
      try {
        text = await readFile(authFile, 'utf8')
      } catch {
        throw new CodexAuthError(
          'auth_missing',
          `No Codex auth at ${authFile}. Run codex login or beginLogin first.`
        )
      }
      return importFromAuthJson(text, `vendor:${authFile}`)
    },
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

function labelFromCodexAuth(
  auth: CodexAuthDotJson
): {
  label: string;
  identityKey: string | null;
  detail: string | null
} {
  if (nonEmptyString(auth.OPENAI_API_KEY)) {
    return { label: 'OPENAI_API_KEY', identityKey: 'apiKey', detail: 'apiKey' }
  }
  const pat = nonEmptyString(auth.personal_access_token)
  if (pat) {
    const claims = parseChatGptClaims(pat)
    const label = claims.email ?? claims.accountId ?? 'personal access token'
    return {
      label,
      identityKey: claims.accountId ?? label,
      detail: 'personalAccessToken'
    }
  }
  const tokens = auth.tokens
  if (tokens) {
    const access = nonEmptyString(tokens.access_token)
    const idClaims = parseIdTokenClaims(tokens.id_token)
    const accessClaims = access ? parseChatGptClaims(access) : {
      accountId: null,
      email: null,
      isFedrampAccount: false
    }
    const accountId = nonEmptyString(tokens.account_id) ?? idClaims.accountId
      ?? accessClaims.accountId
    const email = idClaims.email ?? accessClaims.email
    const label = email ?? accountId ?? 'chatgpt'
    return { label, identityKey: accountId ?? email, detail: 'chatgpt' }
  }
  return { label: 'codex', identityKey: null, detail: null }
}

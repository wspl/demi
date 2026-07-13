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
  parseIdTokenClaims,
  type CodexAuthDotJson,
  type CodexAuthStore,
  type FileCodexAuthStoreOptions,
} from './auth'
import { runCodexDeviceLogin } from './device-login'
import {
  FileCredentialPool,
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
 * Auth store that prefers the demi pool active entry; falls back to vendor ~/.codex.
 */
export class PoolAwareCodexAuthStore implements CodexAuthStore {
  private readonly pool: FileCredentialPool
  private readonly vendorHome: string
  private readonly fileAuthOptions: Omit<FileCodexAuthStoreOptions, 'codexHome' | 'authFile'>

  constructor(
    pool: FileCredentialPool,
    options: { codexHome?: string; fileAuthOptions?: Omit<FileCodexAuthStoreOptions, 'codexHome' | 'authFile'> } = {},
  ) {
    this.pool = pool
    this.vendorHome = options.codexHome ?? defaultCodexHome()
    this.fileAuthOptions = options.fileAuthOptions ?? {}
  }

  async status() {
    return this.currentStore().then((s) => s.status())
  }

  async resolveAuth(options?: { forceRefresh?: boolean }) {
    return this.currentStore().then((s) => s.resolveAuth(options))
  }

  private async currentStore(): Promise<FileCodexAuthStore> {
    await this.pool.ensureActivePointer()
    const activeId = await this.pool.getActiveId()
    if (activeId) {
      return new FileCodexAuthStore({
        ...this.fileAuthOptions,
        authFile: this.pool.secretPath(activeId),
        codexHome: this.pool.entryDir(activeId),
      })
    }
    return new FileCodexAuthStore({
      ...this.fileAuthOptions,
      codexHome: this.vendorHome,
    })
  }
}

export function createCodexCredentials(
  pool: FileCredentialPool,
  authStore: CodexAuthStore,
  options: {
    codexHome?: string
    quota?: ProviderQuota | null
    onActiveChange?: () => void
    /** Injectable fetch for the device-code login flow (tests). */
    loginFetch?: typeof fetch
  } = {},
): ProviderCredentials {
  const vendorHome = options.codexHome ?? defaultCodexHome()

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
    options.onActiveChange?.()
    return getActive()
  }

  const importFromAuthJson = async (authText: string, source: string): Promise<ProviderCredentialInfo> => {
    let auth: CodexAuthDotJson
    try {
      auth = JSON.parse(authText) as CodexAuthDotJson
    } catch {
      throw new CodexAuthError('auth_invalid', 'Codex auth material is not valid JSON')
    }
    if (!isRecord(auth)) throw new CodexAuthError('auth_invalid', 'Codex auth material is not an object')

    const { label, identityKey, detail } = labelFromCodexAuth(auth)
    const existing = identityKey ? await pool.findByIdentityKey(identityKey) : null
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
    if (!active) await pool.setActiveId(id)
    options.quota?.clearLatest?.()
    options.onActiveChange?.()
    return { id: meta.id, label: meta.label, detail: meta.detail, updatedAt: meta.updatedAt }
  }

  return {
    capability,
    list: () => pool.list(),
    getActive,
    setActive,
    // Native device-code flow: pending material streams out via onPending, the completed
    // material never touches the vendor home and is imported straight into the pool.
    beginLogin: async (loginOptions?: ProviderCredentialLoginOptions): Promise<ProviderCredentialLoginResult> => {
      try {
        const auth = await runCodexDeviceLogin({
          signal: loginOptions?.signal,
          onPending: loginOptions?.onPending,
          fetch: options.loginFetch,
        })
        const info = await importFromAuthJson(`${JSON.stringify(auth, null, 2)}\n`, 'login:device')
        return { status: 'completed', credentialId: info.id }
      } catch (error) {
        if (loginOptions?.signal?.aborted) return { status: 'cancelled' }
        if (error instanceof CodexAuthError && error.code === 'auth_unsupported') {
          return { status: 'unavailable', message: error.message }
        }
        return { status: 'failed', message: errorMessage(error) }
      }
    },
    importDefault: async () => {
      const authFile = join(vendorHome, 'auth.json')
      let text: string
      try {
        text = await readFile(authFile, 'utf8')
      } catch {
        throw new CodexAuthError('auth_missing', `No Codex auth at ${authFile}. Run codex login or beginLogin first.`)
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
      if (isRecord(input.auth) || isRecord(input.authJson)) {
        const obj = (input.auth ?? input.authJson) as CodexAuthDotJson
        return importFromAuthJson(`${JSON.stringify(obj, null, 2)}\n`, 'add:auth')
      }
      throw new Error('Codex credentials.add expects authJsonText, authFile, or auth/authJson object')
    },
    remove: async (credentialId: string) => {
      await pool.remove(credentialId)
      options.quota?.clearLatest?.()
      options.onActiveChange?.()
    },
  }
}

export function openCodexCredentialPool(options: { stateDir?: string } = {}): FileCredentialPool {
  return new FileCredentialPool({
    stateDir: options.stateDir,
    providerKey: 'codex',
    secretFileName: 'auth.json',
  })
}

function labelFromCodexAuth(auth: CodexAuthDotJson): { label: string; identityKey: string | null; detail: string | null } {
  if (nonEmptyString(auth.OPENAI_API_KEY)) {
    return { label: 'OPENAI_API_KEY', identityKey: 'apiKey', detail: 'apiKey' }
  }
  const pat = nonEmptyString(auth.personal_access_token)
  if (pat) {
    const claims = parseChatGptClaims(pat)
    const label = claims.email ?? claims.accountId ?? 'personal access token'
    return { label, identityKey: claims.accountId ?? label, detail: 'personalAccessToken' }
  }
  const tokens = auth.tokens
  if (tokens && typeof tokens === 'object') {
    const access = nonEmptyString(tokens.access_token)
    const idClaims = parseIdTokenClaims(tokens.id_token)
    const accessClaims = access ? parseChatGptClaims(access) : { accountId: null, email: null, isFedrampAccount: false }
    const accountId = nonEmptyString(tokens.account_id) ?? idClaims.accountId ?? accessClaims.accountId
    const email = idClaims.email ?? accessClaims.email
    const label = email ?? accountId ?? 'chatgpt'
    return { label, identityKey: accountId ?? email, detail: 'chatgpt' }
  }
  return { label: 'codex', identityKey: null, detail: null }
}

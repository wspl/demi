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
import { errorMessage } from '@demicodes/utils'
import { claudeCredentialAddSchema, parseClaudeAuthData } from './auth-schemas'
import { createHash } from 'node:crypto'
import {
  FileClaudeCodeAuthStore,
  type ClaudeCodeAuthStore,
} from './auth'
import {
  refreshClaudeCodeSecret,
  runClaudeCodeLogin,
  type ClaudeCodeOAuthSecret
} from './login'
import {
  FileCredentialPool,
  credentialIdFromIdentity,
  type CredentialEntryMeta,
} from '@demicodes/provider/credentials-pool'
import type { ClaudeCodeOAuthAccess } from './oauth'

export class PoolAwareClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  constructor(private readonly pool: FileCredentialPool) {}

  async status() {
    return this.currentStore().then((s) => s.status())
  }

  async resolveAccess(options?: { forceRefresh?: boolean }) {
    return this.currentStore().then((s) => s.resolveAccess(options))
  }

  private async currentStore(): Promise<FileClaudeCodeAuthStore> {
    await this.pool.ensureActivePointer()
    const activeId = await this.pool.getActiveId()
    if (activeId) {
      return new FileClaudeCodeAuthStore({
        oauthFile: this.pool.secretPath(activeId),
        refresh: refreshClaudeCodeSecret,
      })
    }
    return new FileClaudeCodeAuthStore()
  }
}

export function openClaudeCodeCredentialPool(
  options: { stateDir?: string } = {}
): FileCredentialPool {
  return new FileCredentialPool({
    stateDir: options.stateDir,
    providerKey: 'claude-code',
    secretFileName: 'oauth.json',
  })
}

export function createClaudeCodeCredentials(
  pool: FileCredentialPool,
  authStore: ClaudeCodeAuthStore,
  options: {
    quota?: ProviderQuota | null
    onActiveChange?: () => void
    /** Injectable fetch for the OAuth login flow (tests). */
    loginFetch?: typeof fetch
    resolveDefaultAccess?: () => Promise<ClaudeCodeOAuthAccess>
  } = {},
): ProviderCredentials {

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
    options.onActiveChange?.()
    return getActive()
  }

  const importSecret = async (
    secret: ClaudeCodeOAuthSecret,
    source: string
  ): Promise<ProviderCredentialInfo> => {
    const email = secret.emailAddress
    const identityKey = email
      ? `email:${email}`
      : `token:${createHash('sha256').update(secret.accessToken).digest('hex').slice(0, 16)}`
    const label = email ?? secret.subscriptionType
      ?? `claude-${identityKey.slice(-8)}`
    const existing = await pool.findByIdentityKey(identityKey)
    const id = existing?.id ?? credentialIdFromIdentity(identityKey, label)
    const meta: CredentialEntryMeta = {
      id,
      label,
      detail: secret.subscriptionType
        ?? secret.rateLimitTier
        ?? null,
      updatedAt: new Date().toISOString(),
      source,
      identityKey,
    }
    await pool.writeEntry(meta, `${JSON.stringify(secret, null, 2)}\n`)
    const active = await pool.getActiveId()
    if (!active)
      await pool.setActiveId(id)
    options.quota?.clearLatest?.()
    options.onActiveChange?.()
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
    // Native copy-back OAuth flow: onPending carries the authorize URL, promptForCode
    // collects the "code#state" string the vendor page shows after approval, and the
    // refreshable secret is imported straight into the pool.
    beginLogin: async (
      loginOptions?: ProviderCredentialLoginOptions
    ): Promise<ProviderCredentialLoginResult> => {
      if (!loginOptions?.promptForCode) {
        return {
          status: 'unavailable',
          message: 'Claude login requires promptForCode to collect the pasted authorization code'
        }
      }
      try {
        const secret = await runClaudeCodeLogin({
          signal: loginOptions.signal,
          onPending: loginOptions.onPending,
          promptForCode: loginOptions.promptForCode,
          fetch: options.loginFetch,
        })
        const info = await importSecret(secret, 'login:oauth')
        return { status: 'completed', credentialId: info.id }
      } catch (error) {
        if (loginOptions.signal?.aborted)
          return { status: 'cancelled' }
        return { status: 'failed', message: errorMessage(error) }
      }
    },
    importDefault: async () => {
      const access = await (options.resolveDefaultAccess
        ? options.resolveDefaultAccess()
        : new FileClaudeCodeAuthStore().resolveAccess())
      return importSecret({
        accessToken: access.accessToken,
        subscriptionType: access.subscriptionType,
        rateLimitTier: access.rateLimitTier,
      }, 'vendor:default')
    },
    add: async (input: ProviderCredentialAddInput) => {
      const parsed = parseClaudeAuthData(claudeCredentialAddSchema, input, 'Claude credential input')
      if ('oauth' in parsed) {
        return importSecret(parsed.oauth, 'add:oauth')
      }
      return importSecret(parsed, 'add:accessToken')
    },
    remove: async (credentialId: string) => {
      await pool.remove(credentialId)
      options.quota?.clearLatest?.()
      options.onActiveChange?.()
    },
  }
}

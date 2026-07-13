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
import { createHash } from 'node:crypto'
import {
  ClaudeCodeAuthError,
  FileClaudeCodeAuthStore,
  type ClaudeCodeAuthStore,
} from './auth'
import { refreshClaudeCodeSecret, runClaudeCodeLogin, type ClaudeCodeOAuthSecret } from './login'
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

  async resolveAccess() {
    return this.currentStore().then((s) => s.resolveAccess())
  }

  private async currentStore(): Promise<FileClaudeCodeAuthStore> {
    await this.pool.ensureActivePointer()
    const activeId = await this.pool.getActiveId()
    if (activeId) {
      return new FileClaudeCodeAuthStore({
        oauthFile: this.pool.secretPath(activeId),
        refresh: (secret) => refreshClaudeCodeSecret(secret as ClaudeCodeOAuthSecret),
      })
    }
    return new FileClaudeCodeAuthStore()
  }
}

export function openClaudeCodeCredentialPool(options: { stateDir?: string } = {}): FileCredentialPool {
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

  const setActive = async (credentialId: string): Promise<ProviderCredentialActive> => {
    await pool.setActiveId(credentialId)
    options.quota?.clearLatest?.()
    options.onActiveChange?.()
    return getActive()
  }

  const importAccess = async (access: ClaudeCodeOAuthAccess, source: string): Promise<ProviderCredentialInfo> => {
    const token = nonEmptyString(access.accessToken)
    if (!token) throw new ClaudeCodeAuthError('auth_missing', 'No Claude access token to import')
    const identityKey =
      nonEmptyString(access.subscriptionType) != null
        ? `token:${createHash('sha256').update(token).digest('hex').slice(0, 16)}:${access.subscriptionType}`
        : `token:${createHash('sha256').update(token).digest('hex').slice(0, 16)}`
    const label = nonEmptyString(access.subscriptionType) ?? `claude-${identityKey.slice(-8)}`
    const existing = await pool.findByIdentityKey(identityKey)
    const id = existing?.id ?? credentialIdFromIdentity(identityKey, label)
    const meta: CredentialEntryMeta = {
      id,
      label,
      detail: nonEmptyString(access.rateLimitTier) ?? null,
      updatedAt: new Date().toISOString(),
      source,
      identityKey,
    }
    const secret = {
      accessToken: token,
      subscriptionType: access.subscriptionType ?? null,
      rateLimitTier: access.rateLimitTier ?? null,
    }
    await pool.writeEntry(meta, `${JSON.stringify(secret, null, 2)}\n`)
    const active = await pool.getActiveId()
    if (!active) await pool.setActiveId(id)
    options.quota?.clearLatest?.()
    options.onActiveChange?.()
    return { id: meta.id, label: meta.label, detail: meta.detail, updatedAt: meta.updatedAt }
  }

  const importSecret = async (secret: ClaudeCodeOAuthSecret, source: string): Promise<ProviderCredentialInfo> => {
    const email = nonEmptyString(secret.emailAddress)
    const identityKey = email
      ? `email:${email}`
      : `token:${createHash('sha256').update(secret.accessToken).digest('hex').slice(0, 16)}`
    const label = email ?? nonEmptyString(secret.subscriptionType) ?? `claude-${identityKey.slice(-8)}`
    const existing = await pool.findByIdentityKey(identityKey)
    const id = existing?.id ?? credentialIdFromIdentity(identityKey, label)
    const meta: CredentialEntryMeta = {
      id,
      label,
      detail: nonEmptyString(secret.subscriptionType) ?? nonEmptyString(secret.rateLimitTier) ?? null,
      updatedAt: new Date().toISOString(),
      source,
      identityKey,
    }
    await pool.writeEntry(meta, `${JSON.stringify(secret, null, 2)}\n`)
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
    // Native copy-back OAuth flow: onPending carries the authorize URL, promptForCode
    // collects the "code#state" string the vendor page shows after approval, and the
    // refreshable secret is imported straight into the pool.
    beginLogin: async (loginOptions?: ProviderCredentialLoginOptions): Promise<ProviderCredentialLoginResult> => {
      if (!loginOptions?.promptForCode) {
        return { status: 'unavailable', message: 'Claude login requires promptForCode to collect the pasted authorization code' }
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
        if (loginOptions.signal?.aborted) return { status: 'cancelled' }
        return { status: 'failed', message: errorMessage(error) }
      }
    },
    importDefault: async () => {
      const vendor = new FileClaudeCodeAuthStore()
      let access: ClaudeCodeOAuthAccess
      try {
        access = await vendor.resolveAccess()
      } catch {
        throw new ClaudeCodeAuthError(
          'auth_missing',
          'No Claude Code OAuth to import. Run claude auth login or beginLogin first.',
        )
      }
      return importAccess(access, 'vendor:default')
    },
    add: async (input: ProviderCredentialAddInput) => {
      if (typeof input.accessToken === 'string') {
        return importAccess(
          {
            accessToken: input.accessToken,
            source: 'static',
            subscriptionType: typeof input.subscriptionType === 'string' ? input.subscriptionType : null,
            rateLimitTier: typeof input.rateLimitTier === 'string' ? input.rateLimitTier : null,
          },
          'add:accessToken',
        )
      }
      if (isRecord(input.oauth) && typeof input.oauth.accessToken === 'string') {
        const oauth = input.oauth
        return importAccess(
          {
            accessToken: oauth.accessToken as string,
            source: 'static',
            subscriptionType: typeof oauth.subscriptionType === 'string' ? oauth.subscriptionType : null,
            rateLimitTier: typeof oauth.rateLimitTier === 'string' ? oauth.rateLimitTier : null,
          },
          'add:oauth',
        )
      }
      throw new Error('Claude credentials.add expects accessToken or oauth.accessToken')
    },
    remove: async (credentialId: string) => {
      await pool.remove(credentialId)
      options.quota?.clearLatest?.()
      options.onActiveChange?.()
    },
  }
}

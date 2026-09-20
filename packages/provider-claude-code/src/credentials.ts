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
import { createHash } from 'node:crypto'
import { z } from 'zod'
import {
  ClaudeCodeAuthError,
  FileClaudeCodeAuthStore,
  type ClaudeCodeAuthStore,
} from './auth'
import { refreshClaudeCodeSecret, runClaudeCodeLogin } from './login'
import type { ClaudeCodeOAuthSecret } from './secret'
import {
  FileCredentialPool,
  type CredentialPool,
  credentialIdFromIdentity,
  type CredentialEntryMeta,
} from '@demicodes/provider/credentials-pool'
import type { ClaudeCodeOAuthAccess } from './oauth'

/**
 * Auth store that prefers the demi pool active entry; falls back to the vendor
 * login of this machine where the pool allows it.
 */
export class PoolAwareClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  private readonly pool: CredentialPool
  private readonly credentialId: string | null

  constructor(
    pool: CredentialPool,
    options: {
      /** The account this store stands for, instead of the pool's active one. */
      credentialId?: string
    } = {},
  ) {
    this.pool = pool
    this.credentialId = options.credentialId ?? null
  }

  async status() {
    return this.currentStore().then((s) => s.status())
  }

  async resolveAccess() {
    return this.currentStore().then((s) => s.resolveAccess())
  }

  private async currentStore(): Promise<ClaudeCodeAuthStore> {
    const id = this.credentialId ?? await this.pool.ensureActivePointer()
    if (id) {
      return new FileClaudeCodeAuthStore({
        document: this.pool.document(id),
        refresh: refreshClaudeCodeSecret,
      })
    }
    // The environment token and the keychain are the login of whoever runs
    // this machine: only a pool that stands for that user may read them.
    if (!this.pool.vendorDefault)
      return new MissingClaudeCodeAuthStore()
    return new FileClaudeCodeAuthStore()
  }
}

/** An entry without an account, where no vendor login may stand in for one. */
class MissingClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  async status() {
    return {
      status: 'unauthenticated' as const,
      message: 'No Claude Code account is signed in'
    }
  }

  async resolveAccess(): Promise<never> {
    throw new ClaudeCodeAuthError(
      'auth_missing',
      'No Claude Code account is signed in'
    )
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

/**
 * The material `credentials.add` accepts, given directly or under `oauth`.
 */
const credentialMaterialSchema = z.looseObject({
  accessToken: z.string().min(1),
  subscriptionType: z.string().min(1).optional(),
  rateLimitTier: z.string().min(1).optional(),
})
type CredentialMaterial = z.infer<typeof credentialMaterialSchema>

const nestedCredentialMaterialSchema = z.looseObject({
  oauth: credentialMaterialSchema,
})

function accessFromMaterial(
  material: CredentialMaterial
): ClaudeCodeOAuthAccess {
  return {
    accessToken: material.accessToken,
    source: 'static',
    subscriptionType: material.subscriptionType ?? null,
    rateLimitTier: material.rateLimitTier ?? null,
  }
}

export function createClaudeCodeCredentials(
  pool: CredentialPool,
  authStore: ClaudeCodeAuthStore,
  options: {
    quota?: ProviderQuota | null
    /**
     * The provider stands for one named account: another account becoming
     * active or being added says nothing about its usage.
     */
    pinned?: boolean
    onActiveChange?: () => void
    /** Injectable fetch for the OAuth login flow (tests). */
    loginFetch?: typeof fetch
  } = {},
): ProviderCredentials {
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

  const importAccess = async (
    access: ClaudeCodeOAuthAccess,
    source: string
  ): Promise<ProviderCredentialInfo> => {
    const token = nonEmptyString(access.accessToken)
    if (!token)
      throw new ClaudeCodeAuthError(
        'auth_missing',
        'No Claude access token to import'
      )
    const identityKey =
      nonEmptyString(access.subscriptionType) != null
        ? `token:${createHash('sha256').update(token).digest('hex').slice(0, 16)}:${access.subscriptionType}`
        : `token:${createHash('sha256').update(token).digest('hex').slice(0, 16)}`
    const label = nonEmptyString(access.subscriptionType)
      ?? `claude-${identityKey.slice(-8)}`
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

  const importSecret = async (
    secret: ClaudeCodeOAuthSecret,
    source: string
  ): Promise<ProviderCredentialInfo> => {
    const email = nonEmptyString(secret.emailAddress)
    const identityKey = email
      ? `email:${email}`
      : `token:${createHash('sha256').update(secret.accessToken).digest('hex').slice(0, 16)}`
    const label = email ?? nonEmptyString(secret.subscriptionType)
      ?? `claude-${identityKey.slice(-8)}`
    const existing = await pool.findByIdentityKey(identityKey)
    const id = existing?.id ?? credentialIdFromIdentity(identityKey, label)
    const meta: CredentialEntryMeta = {
      id,
      label,
      detail: nonEmptyString(secret.subscriptionType)
        ?? nonEmptyString(secret.rateLimitTier)
        ?? null,
      updatedAt: new Date().toISOString(),
      source,
      identityKey,
    }
    await pool.writeEntry(meta, `${JSON.stringify(secret, null, 2)}\n`)
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
      if (!pool.vendorDefault)
        throw new ClaudeCodeAuthError(
          'auth_unsupported',
          'This pool does not take the vendor login of the machine'
        )
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
      const direct = credentialMaterialSchema.safeParse(input)
      if (direct.success) {
        return importAccess(
          accessFromMaterial(direct.data),
          'add:accessToken'
        )
      }
      const nested = nestedCredentialMaterialSchema.safeParse(input)
      if (nested.success) {
        return importAccess(accessFromMaterial(nested.data.oauth), 'add:oauth')
      }
      throw new Error(
        'Claude credentials.add expects accessToken or oauth.accessToken'
      )
    },
    remove: async (credentialId: string) => {
      await pool.remove(credentialId)
      accountChanged()
    },
  }
}

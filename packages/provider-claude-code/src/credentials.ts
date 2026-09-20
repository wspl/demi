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
import { createHash } from 'node:crypto'
import { z } from 'zod'
import {
  ClaudeCodeAuthError,
  ClaudeCodeDocumentAuthStore,
  parseClaudeCodeOAuthSecret,
  type ClaudeCodeAuthStore,
  type ClaudeCodeSecretRefresh,
} from './auth'
import { refreshClaudeCodeSecret, runClaudeCodeLogin } from './login'
import type { ClaudeCodeOAuthSecret } from './secret'
import {
  claudeCodeAccessSource,
  claudeCodeVendorPool,
  labelFromClaudeCodeToken,
  type ClaudeCodeTokenSecret,
  type ClaudeCodeVendorOptions,
} from './vendor'
import {
  FileCredentialPool,
  type CredentialPool,
  credentialIdFromIdentity,
  type CredentialEntryMeta,
} from '@demicodes/provider/credentials-pool'
import type { ClaudeCodeOAuthAccess } from './oauth'

/**
 * The auth store of a pool: the account it was built for, or the one the pool
 * has active. Which keeper the pool is makes no difference here; the account's
 * meta says what its access reports as its source.
 */
export class PoolAwareClaudeCodeAuthStore implements ClaudeCodeAuthStore {
  constructor(
    private readonly pool: CredentialPool,
    private readonly options: {
      /** The account this store stands for, instead of the pool's active one. */
      credentialId?: string
      refresh?: ClaudeCodeSecretRefresh
    } = {},
  ) {}

  async status(): Promise<ProviderAuthState> {
    try {
      return await (await this.currentStore()).status()
    } catch (error) {
      // A pool that cannot be read is a state to report, like a store's own.
      return { status: 'error', message: errorMessage(error) }
    }
  }

  async resolveAccess() {
    return this.currentStore().then((s) => s.resolveAccess())
  }

  private async currentStore(): Promise<ClaudeCodeAuthStore> {
    const id = this.options.credentialId
      ?? await this.pool.ensureActivePointer()
    if (!id)
      return new MissingClaudeCodeAuthStore()
    return new ClaudeCodeDocumentAuthStore({
      document: this.pool.document(id),
      source: claudeCodeAccessSource((await this.pool.readMeta(id))?.source),
      refresh: this.options.refresh ?? refreshClaudeCodeSecret,
    })
  }
}

/** A pool without an account. */
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

/** The access of this machine's vendor login; null when there is none. */
export async function resolveClaudeCodeOAuthAccess(
  vendor: ClaudeCodeVendorOptions = {}
): Promise<ClaudeCodeOAuthAccess | null> {
  try {
    return await new PoolAwareClaudeCodeAuthStore(claudeCodeVendorPool(vendor))
      .resolveAccess()
  } catch {
    return null
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

const nestedCredentialMaterialSchema = z.looseObject({
  oauth: credentialMaterialSchema,
})

export function createClaudeCodeCredentials(
  pool: CredentialPool,
  authStore: ClaudeCodeAuthStore,
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
    /** Injectable fetch for the OAuth login flow (tests). */
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

  const importToken = async (
    material: ClaudeCodeTokenSecret,
    source: string
  ): Promise<ProviderCredentialInfo> => {
    const { label, identityKey, detail } = labelFromClaudeCodeToken(material)
    const existing = await pool.findByIdentityKey(identityKey)
    const id = existing?.id ?? credentialIdFromIdentity(identityKey, label)
    const meta: CredentialEntryMeta = {
      id,
      label,
      detail,
      updatedAt: new Date().toISOString(),
      source,
      identityKey,
    }
    const secret = {
      accessToken: material.accessToken,
      subscriptionType: material.subscriptionType ?? null,
      rateLimitTier: material.rateLimitTier ?? null,
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
    ...(options.importFrom ? {
      importDefault: async () => {
        const from = options.importFrom!
        const id = await from.ensureActivePointer()
        const revision = id ? await from.document(id).read() : null
        if (!id || !revision)
          throw new ClaudeCodeAuthError(
            'auth_missing',
            'No Claude Code OAuth to import. Run claude auth login or beginLogin first.',
          )
        // A copied token is a stored secret: its source names the import, not
        // the keeper it came from.
        return importToken(
          parseClaudeCodeOAuthSecret(revision.text, from.document(id).name),
          'vendor:default'
        )
      },
    } : {}),
    add: async (input: ProviderCredentialAddInput) => {
      const direct = credentialMaterialSchema.safeParse(input)
      if (direct.success) {
        return importToken(direct.data, 'add:accessToken')
      }
      const nested = nestedCredentialMaterialSchema.safeParse(input)
      if (nested.success) {
        return importToken(nested.data.oauth, 'add:oauth')
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

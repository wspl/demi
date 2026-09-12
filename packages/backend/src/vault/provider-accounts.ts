import { rename, rm } from 'node:fs/promises'
import { createId } from '@demicodes/utils'
import type { ProviderCredentials } from '@demicodes/provider'
import type { ProviderAssembly } from '../llm/assembly'
import type { ProviderEntry, ProviderVault } from './providers'

export class AccountRefused extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly status: 400 | 404 | 409 = 409
  ) {
    super(message)
  }
}

/**
 * Product rules over the framework credential pool: token import, explicit
 * active selection, and safe removal.
 */
export class ProviderAccounts {
  constructor(
    private readonly vault: ProviderVault,
    private readonly assembly: ProviderAssembly
  ) {}

  async importClaude(
    ownerUserId: string | null,
    label: string,
    token: string
  ): Promise<ProviderEntry> {
    if ((await this.vault.list({ ownerUserId })).some(
      entry => entry.config.providerType === 'claude-code'
    )) {
      throw new AccountRefused(
        'provider_exists',
        'Add this token to the existing Claude provider'
      )
    }
    const id = createId()
    const pendingDir = this.assembly.vaultDir(`pending-${id}`)
    let unpublishedDir: string | null = pendingDir
    try {
      const provider = this.assembly.buildDetached(
        'claude-code',
        { id, label, vaultDir: pendingDir }
      )
      if (!provider.credentials?.add)
        throw new AccountRefused(
          'unsupported',
          'Token import is unavailable',
          400
        )
      await provider.credentials.add({ accessToken: token })
      const destination = this.assembly.vaultDir(id)
      await rename(pendingDir, destination)
      unpublishedDir = destination
      const entry = await this.vault.create({
        id,
        ownerUserId,
        label,
        config: {
          kind: 'subscription',
          providerType: 'claude-code'
        }
      })
      unpublishedDir = null
      return entry
    } catch (error) {
      if (error instanceof AccountRefused)
        throw error
      // Credential import failures may contain supplied token material; only a safe product error crosses HTTP.
      throw new AccountRefused(
        'token_import_failed',
        'Unable to import setup token'
      )
    } finally {
      if (unpublishedDir)
        await rm(unpublishedDir, { recursive: true, force: true })
    }
  }

  async list(entry: ProviderEntry) {
    const credentials = await this.credentials(entry)
    return {
      accounts: await credentials.list(),
      active: await credentials.getActive()
    }
  }

  async addToken(entry: ProviderEntry, token: string) {
    if (entry.config.providerType !== 'claude-code')
      throw new AccountRefused(
        'unsupported',
        'Use device login for this provider',
        400
      )
    const credentials = await this.credentials(entry)
    if (!credentials.add)
      throw new AccountRefused(
        'unsupported',
        'Token import is unavailable',
        400
      )
    try {
      const account = await credentials.add({ accessToken: token })
      await this.assembly.invalidate(entry.id)
      return { account }
    } catch {
      throw new AccountRefused(
        'token_import_failed',
        'Unable to import setup token'
      )
    }
  }

  async activate(entry: ProviderEntry, id: string) {
    const credentials = await this.existingAccount(entry, id)
    const active = await credentials.setActive(id)
    await this.assembly.invalidate(entry.id)
    return { active }
  }

  async remove(entry: ProviderEntry, id: string): Promise<void> {
    const credentials = await this.existingAccount(entry, id)
    if (!credentials.remove)
      throw new AccountRefused(
        'unsupported',
        'Account removal is unavailable',
        400
      )
    if ((await credentials.getActive()).credentialId === id)
      throw new AccountRefused(
        'active_account',
        'Switch accounts before removing the active account, or remove the provider'
      )
    await credentials.remove(id)
    await this.assembly.invalidate(entry.id)
  }

  private async credentials(
    entry: ProviderEntry
  ): Promise<ProviderCredentials> {
    const resolved = await this.assembly.providerFor(entry.id)
    if (entry.config.kind !== 'subscription' || !resolved?.provider.credentials)
      throw new AccountRefused(
        'unsupported',
        'This provider does not use subscription accounts',
        400
      )
    return resolved.provider.credentials
  }

  private async existingAccount(
    entry: ProviderEntry,
    id: string
  ): Promise<ProviderCredentials> {
    const credentials = await this.credentials(entry)
    if (!(await credentials.list()).some(account => account.id === id))
      throw new AccountRefused('account_not_found', 'No such account', 404)
    return credentials
  }
}

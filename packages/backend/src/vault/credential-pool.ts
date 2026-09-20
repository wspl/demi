import {
  CredentialPoolError,
  MemoryCredentialPool,
  type CredentialDocument,
  type CredentialEntryMeta,
  type CredentialPool,
} from '@demicodes/provider/credentials-pool'
import {
  storedQuotaSnapshotSchema,
  type ProviderCredentialInfo,
  type ProviderQuotaSnapshot,
  type ProviderQuotaSnapshots,
} from '@demicodes/provider'
import type {
  ControlService,
  ProviderCredentialRecord,
  ProviderCredentialWrite
} from '../storage/control'
import { decryptJson, encryptJson } from './crypto'

function metaOf(record: ProviderCredentialRecord): CredentialEntryMeta {
  return {
    id: record.id,
    label: record.label,
    detail: record.detail,
    updatedAt: record.updatedAt,
    source: record.source,
    identityKey: record.identityKey,
  }
}

function infoOf(meta: CredentialEntryMeta): ProviderCredentialInfo {
  return {
    id: meta.id,
    label: meta.label,
    detail: meta.detail ?? null,
    updatedAt: meta.updatedAt,
  }
}

function writeOf(
  secret: Uint8Array,
  meta: CredentialEntryMeta,
  secretText: string
): ProviderCredentialWrite {
  return {
    id: meta.id,
    identityKey: meta.identityKey ?? null,
    label: meta.label,
    detail: meta.detail ?? null,
    source: meta.source ?? null,
    secret: encryptJson(secret, secretText),
  }
}

/**
 * One subscription entry's accounts as control records
 * (providers-and-vault.md § Credential vault). Bound to the entry: a provider
 * package given this pool cannot name another entry's accounts, and an entry
 * without an account never stands for a vendor login of this machine.
 */
export class VaultCredentialPool implements CredentialPool {
  readonly vendorDefault = false

  constructor(
    private readonly control: ControlService,
    private readonly secret: Uint8Array,
    private readonly providerId: string,
  ) {}

  async list(): Promise<ProviderCredentialInfo[]> {
    return (await this.listMeta()).map(infoOf)
  }

  async listMeta(): Promise<CredentialEntryMeta[]> {
    return (await this.control.listProviderCredentials(this.providerId))
      .map(metaOf)
  }

  async readMeta(id: string): Promise<CredentialEntryMeta | null> {
    const record = await this.control.getProviderCredential(this.providerId, id)
    return record ? metaOf(record) : null
  }

  async findByIdentityKey(
    identityKey: string
  ): Promise<CredentialEntryMeta | null> {
    return (await this.listMeta())
      .find(meta => meta.identityKey === identityKey) ?? null
  }

  async getActiveId(): Promise<string | null> {
    return (await this.control.getProvider(this.providerId))
      ?.activeCredentialId ?? null
  }

  async setActiveId(id: string): Promise<void> {
    if (!await this.control.setActiveProviderCredential(this.providerId, id))
      throw new CredentialPoolError(
        'credential_not_found',
        `Credential "${id}" not found`
      )
  }

  async ensureActivePointer(): Promise<string | null> {
    const active = await this.getActiveId()
    if (active)
      return active
    const first = (await this.listMeta())[0]
    if (!first)
      return null
    await this.setActiveId(first.id)
    return first.id
  }

  async writeEntry(
    meta: CredentialEntryMeta,
    secretText: string
  ): Promise<CredentialEntryMeta> {
    return metaOf(await this.control.putProviderCredential(
      this.providerId,
      writeOf(this.secret, meta, secretText)
    ))
  }

  document(id: string): CredentialDocument {
    return {
      key: `provider_credentials/${this.providerId}/${id}`,
      name: `account ${id}`,
      read: async () => {
        const record = await this.control.getProviderCredential(
          this.providerId,
          id
        )
        if (!record)
          return null
        const text = decryptJson(this.secret, record.secret)
        if (typeof text !== 'string')
          throw new Error(`Corrupt credential ${id}: the secret is not a document`)
        return { text, version: String(record.version) }
      },
      replace: (text, version) => this.control.replaceProviderCredentialSecret(
        this.providerId,
        id,
        encryptJson(this.secret, text),
        Number(version)
      ),
    }
  }

  async remove(id: string): Promise<void> {
    await this.control.removeProviderCredential(this.providerId, id)
  }
}

/**
 * The accounts of an entry that does not exist yet: a login authenticates
 * against memory, and only its completion stores anything.
 */
export class StagedCredentialPool extends MemoryCredentialPool {
  constructor(private readonly secret: Uint8Array) {
    super(false)
  }

  /** What the completed login publishes with the provider row. */
  async accounts(): Promise<{
    credentials: ProviderCredentialWrite[];
    activeCredentialId: string | null
  }> {
    return {
      credentials: this.entries().map(
        entry => writeOf(this.secret, entry.meta, entry.secretText)
      ),
      activeCredentialId: await this.ensureActivePointer(),
    }
  }
}

/**
 * Each account's latest usage snapshot, shared by every provider object built
 * for that account and kept in its record. Keeping is best effort, as the
 * quota contract says.
 */
export class AccountQuotas {
  private readonly held = new Map<string, ProviderQuotaSnapshot | null>()

  constructor(private readonly control: ControlService) {}

  /** The stored snapshot of a record, or null when it holds none it can read. */
  static stored(record: ProviderCredentialRecord): ProviderQuotaSnapshot | null {
    if (!record.quota)
      return null
    try {
      return storedQuotaSnapshotSchema.parse(JSON.parse(record.quota))
    } catch {
      // A snapshot this version cannot read is replaced by the next one.
      return null
    }
  }

  keeper(
    providerId: string,
    record: ProviderCredentialRecord
  ): ProviderQuotaSnapshots {
    const key = `${providerId}/${record.id}`
    if (!this.held.has(key))
      this.held.set(key, AccountQuotas.stored(record))
    return {
      read: () => this.held.get(key) ?? null,
      save: (snapshot) => {
        this.held.set(key, snapshot)
        const { raw: _raw, ...stored } = snapshot ?? { raw: undefined }
        void this.control.setProviderCredentialQuota(
          providerId,
          record.id,
          snapshot ? JSON.stringify(stored) : null
        ).catch(() => undefined)
      },
    }
  }

  /** What is held for an account now, newer than its record may say. */
  latest(
    providerId: string,
    record: ProviderCredentialRecord
  ): ProviderQuotaSnapshot | null {
    const key = `${providerId}/${record.id}`
    return this.held.has(key)
      ? this.held.get(key) ?? null
      : AccountQuotas.stored(record)
  }

  forget(providerId: string): void {
    for (const key of this.held.keys()) {
      if (key.startsWith(`${providerId}/`))
        this.held.delete(key)
    }
  }
}

import { credentialEntryMetaSchema } from '@demicodes/provider/credentials-pool'
import { storedQuotaSnapshotSchema } from '@demicodes/provider'
import { isFileNotFoundError } from '@demicodes/utils'
import { readdir, readFile, rm } from 'node:fs/promises'
import { join } from 'node:path'
import type { ControlService } from '../storage/control'
import { encryptJson } from './crypto'

async function namesIn(dir: string): Promise<string[]> {
  try {
    return await readdir(dir)
  } catch (error) {
    if (isFileNotFoundError(error))
      return []
    throw error
  }
}

async function textOf(path: string): Promise<string | null> {
  try {
    return await readFile(path, 'utf8')
  } catch (error) {
    if (isFileNotFoundError(error))
      return null
    throw error
  }
}

/**
 * Subscription accounts were once pool directories under `<dataDir>/vault/`
 * (providers-and-vault.md § Implementation limits). Each directory of an entry
 * that still exists becomes that entry's account records — secrets, the active
 * selection and the usage snapshot — and is then removed: an account lives in
 * one place. A directory that cannot be read stops the start, since removing
 * it would lose the account.
 */
export async function importCredentialPools(
  control: ControlService,
  secret: Uint8Array,
  vaultRoot: string
): Promise<void> {
  for (const providerId of await namesIn(vaultRoot)) {
    const root = join(vaultRoot, providerId)
    const provider = await control.getProvider(providerId)
    if (provider && (await control.listProviderCredentials(providerId)).length === 0) {
      const pools = join(root, 'credentials')
      for (const providerKey of await namesIn(pools)) {
        const pool = join(pools, providerKey)
        const imported: string[] = []
        for (const entry of await namesIn(join(pool, 'entries'))) {
          const dir = join(pool, 'entries', entry)
          const metaText = await textOf(join(dir, 'meta.json'))
          if (metaText === null)
            continue
          const meta = credentialEntryMetaSchema.parse(JSON.parse(metaText))
          const secretFile = (await namesIn(dir)).find(
            name => name !== 'meta.json' && name.endsWith('.json')
          )
          const secretText = secretFile
            ? await textOf(join(dir, secretFile))
            : null
          if (secretText === null)
            continue
          await control.putProviderCredential(providerId, {
            id: meta.id,
            identityKey: meta.identityKey ?? null,
            label: meta.label,
            detail: meta.detail ?? null,
            source: meta.source ?? null,
            secret: encryptJson(secret, secretText),
          })
          imported.push(meta.id)
        }
        const pointed = (await textOf(join(pool, 'active')))?.trim()
        const active = pointed && imported.includes(pointed)
          ? pointed
          : imported[0]
        if (!active)
          continue
        await control.setActiveProviderCredential(providerId, active)
        const quotaText = await textOf(join(root, 'quota.json'))
        if (quotaText !== null) {
          const quota = storedQuotaSnapshotSchema.safeParse(JSON.parse(quotaText))
          if (quota.success)
            await control.setProviderCredentialQuota(
              providerId,
              active,
              JSON.stringify(quota.data)
            )
        }
      }
    }
    // What remains belongs to a deleted provider, an abandoned login, or was
    // just imported.
    await rm(root, { recursive: true, force: true })
  }
}

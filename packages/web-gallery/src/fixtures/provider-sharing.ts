import type { ProviderShareService } from '@demicodes/web-ui/settings/provider-sharing'
import type { SettingsShareKey } from '@demicodes/web-ui/settings/types'
import { productWould } from '../product-would'
import { simulatedRequestId } from './request-failures'
import { slow } from './slow'
import { DAY, HOUR, MINUTE, ago, ahead } from './time'

/** How long a key nobody used works, as the backend keeps it. */
const UNUSED_FOR = 7 * DAY

/** A key used by one server and one nobody used yet, newest first. */
export const galleryShareKeys = (): SettingsShareKey[] => [
  { id: 'key-unused', createdAt: ago(2 * HOUR), used: null, expiresAt: ahead(UNUSED_FOR - 2 * HOUR) },
  { id: 'key-studio', createdAt: ago(9 * DAY), used: { server: 'studio.example.com', at: ago(12 * MINUTE) }, expiresAt: null },
]

/**
 * Share Provider's service as the product's would answer it, over the keys
 * kept here, each after a beat: New Key makes one, unused for a week, whose
 * text only its answer holds, and Revoke removes one, each said in a neutral
 * toast, since the product's backend does the work. With `failFirst`, the
 * first key fails as a request the backend could not answer, so Try Again has
 * something to retry.
 */
export function useGalleryShareKeys(initial: readonly SettingsShareKey[], options: { failFirst?: boolean } = {}) {
  let failing = options.failFirst ?? false
  let keys = [...initial]
  const service: ProviderShareService = {
    async list() {
      await slow()
      return [...keys]
    },
    async create() {
      await slow()
      if (failing) {
        failing = false
        throw Object.assign(new Error('The request failed (503).'), { code: null, status: 503, requestId: simulatedRequestId('share-key') })
      }
      const key: SettingsShareKey = { id: crypto.randomUUID(), createdAt: new Date().toISOString(), used: null, expiresAt: ahead(UNUSED_FOR) }
      keys = [key, ...keys]
      productWould('Make the Share Key')
      return { key, text: `demi-share-key-${btoa(key.id).replaceAll('=', '')}` }
    },
    async revoke(_providerId, keyId) {
      await slow()
      keys = keys.filter((key) => key.id !== keyId)
      productWould('Revoke the Share Key')
    },
  }
  return { service }
}

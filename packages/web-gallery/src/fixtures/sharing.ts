import { ref } from 'vue'
import { holdsTarget, type ExistingShare, type ShareService } from '@demicodes/web-ui/agent/sharing'
import { productWould } from '../product-would'
import { simulatedRequestId } from './request-failures'
import { slow } from './slow'

/**
 * The share dialog's service as the product's would answer it, over the
 * conversation's `shares` kept here: Share makes a share (or finds the one
 * that exists), Update Link moves it to now and Stop Sharing removes it, each
 * after a beat and said in a neutral toast, since the product's backend does
 * the work. With `failFirst`, the first share fails as a request the backend
 * could not answer, so Try Again has something to retry.
 */
export function useGalleryShares(initial: readonly ExistingShare[] = [], options: { failFirst?: boolean } = {}) {
  let failing = options.failFirst ?? false
  const shares = ref<ExistingShare[]>([...initial])
  const link = (token: string) => `https://demi.example.com/s/${token}`
  const service: ShareService = {
    link,
    async create(target) {
      await slow()
      if (failing) {
        failing = false
        throw Object.assign(new Error('The request failed (503).'), { code: null, status: 503, requestId: simulatedRequestId('share') })
      }
      const held = shares.value.find((share) => holdsTarget(share, target))
      if (held) {
        return { token: held.token, url: link(held.token) }
      }
      const token = crypto.randomUUID().replaceAll('-', '').slice(0, 22)
      shares.value.push(target.kind === 'conversation' ? { kind: 'conversation', token } : { kind: 'answer', token, blockId: target.blockId })
      productWould('Create the Share')
      return { token, url: link(token) }
    },
    async update(token) {
      await slow()
      return { token, url: link(token) }
    },
    async remove(token) {
      await slow()
      shares.value = shares.value.filter((share) => share.token !== token)
      productWould('Remove the Share')
    },
  }
  return { shares, service }
}

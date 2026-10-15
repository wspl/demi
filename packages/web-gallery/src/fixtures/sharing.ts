import { ref } from 'vue'
import { holdsTarget, type ExistingShare, type ShareService } from '@demicodes/web-ui/agent/sharing'
import { productWould } from '../product-would'
import { slow } from './slow'

/**
 * The share dialog's service as the product's would answer it, over the
 * conversation's `shares` kept here: Share makes a share (or finds the one
 * that exists), Update Link takes the snapshot again and Stop Sharing removes
 * it, each after a beat and said in a neutral toast, since the product's
 * backend does the work.
 */
export function useGalleryShares(initial: readonly ExistingShare[] = []) {
  const shares = ref<ExistingShare[]>([...initial])
  const link = (token: string) => `https://demi.example.com/s/${token}`
  const service: ShareService = {
    link,
    async create(target) {
      await slow()
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

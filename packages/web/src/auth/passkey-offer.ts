import { ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { addPasskey, usePasskeys } from './passkeys'

/**
 * The key under which a flow's request for the offer survives the page load
 * it ends with: a handoff lands on its page by loading it. The tab's session
 * storage holds it, so another tab or a later visit is not offered.
 */
const REQUESTED = 'demi.passkeyOffer.requested'

function requestedBefore(): boolean {
  try {
    return sessionStorage.getItem(REQUESTED) !== null
  } catch {
    // Session storage is optional; without it the offer shows only when the
    // flow that asked has not loaded another page.
    return false
  }
}

/**
 * The offer to add a passkey (`users.md` § User system): any flow that has
 * just proved the user calls `request`, a handoff landing now and the
 * password chosen at a first sign-in later; the offer then shows once the
 * account is loaded and passkeys are offered, unless this browser answered
 * it before, and the answer is remembered for the account on this browser
 * (`resources.passkeyOfferAnswered`).
 */
export const usePasskeyOffer = defineStore('passkey-offer', () => {
  const product = useProduct()
  const passkeys = usePasskeys()
  const resources = useResources()
  const requested = ref(requestedBefore())
  /** Whether the card stands: set once by a request, cleared by `close`. */
  const open = ref(false)

  /** Asks for the offer to show for the account that is, or is about to be, signed in. */
  function request(): void {
    requested.value = true
    try {
      sessionStorage.setItem(REQUESTED, '1')
    } catch {
      // See `requestedBefore`.
    }
  }

  watch(
    () => requested.value && product.snapshot !== null && passkeys.offered,
    (ready) => {
      if (ready && !resources.passkeyOfferAnswered) {
        open.value = true
      }
    },
    { immediate: true },
  )

  /** Either button was pressed: this browser was asked. */
  function answered(): void {
    resources.passkeyOfferAnswered = true
  }

  function close(): void {
    open.value = false
    requested.value = false
    try {
      sessionStorage.removeItem(REQUESTED)
    } catch {
      // See `requestedBefore`.
    }
  }

  return { open, request, answered, close, add: addPasskey }
})

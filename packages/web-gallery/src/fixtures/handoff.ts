import { ref } from 'vue'
import type { HandoffCodeView, HandoffHost } from '@demicodes/web-ui/auth/handoff'
import { productWould } from '../product-would'

/** How a specimen's dialog opens: on the confirmation, on a code, on an expired code, or handed off to the phone. */
export type HandoffSpecimenStart = 'verify' | 'code' | 'expired' | 'handedOff'

/** The browser of the stand-in phone the code is handed off to. */
const PHONE = 'Safari on iPhone'

/**
 * A host of Hand Off to Your Phone as the product's would answer, for one
 * specimen: the password `wrong` is refused, any other confirms, as Use
 * Passkey does after the browser's prompt would have been answered; a code
 * lasts ten minutes, and the confirmation five, so asking again within them
 * shows a code at once. `phoneScans` stands in for the phone's camera.
 */
export function galleryHandoff(start: HandoffSpecimenStart): { host: HandoffHost; phoneScans: () => void } {
  /** When the specimen's confirmation holds until; the pinned code states start confirmed. */
  let verifiedUntil = start === 'verify' ? 0 : Date.now() + 5 * 60_000
  let first = true
  let codes = 0
  const handedOff = ref<string | null>(null)
  const handedOffCode = ref<string | null>(null)

  function code(lifetimeMs: number): HandoffCodeView {
    codes += 1
    const id = `handoff-${codes}`
    return {
      id,
      url: `https://demi.example/handoff#gallery-code-${codes}-not-a-real-one`,
      deadline: Date.now() + lifetimeMs,
    }
  }

  const host: HandoffHost = {
    makeCode: async () => {
      await new Promise((resolve) => setTimeout(resolve, 300))
      if (Date.now() >= verifiedUntil) {
        return { kind: 'verify' }
      }
      const opening = first
      first = false
      if (opening && start === 'expired') {
        return { kind: 'code', code: code(-1000) }
      }
      const made = code(10 * 60_000)
      handedOff.value = null
      if (opening && start === 'handedOff') {
        handedOffCode.value = made.id
        handedOff.value = PHONE
      }
      return { kind: 'code', code: made }
    },
    verifyPassword: async (password) => {
      await new Promise((resolve) => setTimeout(resolve, 500))
      if (password === 'wrong') {
        return { kind: 'wrongPassword', message: 'Wrong password' }
      }
      verifiedUntil = Date.now() + 5 * 60_000
      return { kind: 'verified' }
    },
    verifyPasskey: async () => {
      await new Promise((resolve) => setTimeout(resolve, 900))
      productWould('Ask the Browser for a Passkey of This Account')
      verifiedUntil = Date.now() + 5 * 60_000
      return { kind: 'verified' }
    },
    handedOffTo: (id) => (handedOffCode.value === id ? handedOff.value : null),
  }

  function phoneScans(): void {
    handedOffCode.value = `handoff-${codes}`
    handedOff.value = PHONE
  }

  return { host, phoneScans }
}

import { ref } from 'vue'
import type { HandoffCodeView, HandoffHost } from '@demicodes/web-ui/auth/handoff'
import type { VerificationHost } from '@demicodes/web-ui/auth/verification'
import { productWould } from '../product-would'

/**
 * Confirming it is the user as the product's backend would answer, for
 * one specimen:
 *
 * - The password `wrong` is refused and any other confirms, as Use
 *   Passkey does after the browser's prompt would have been answered.
 * - A confirmation holds five minutes.
 * - `verified` says whether one holds now.
 * - `confirmed` starts the specimen with one.
 */
export function galleryVerification(confirmed = false): { host: VerificationHost; verified: () => boolean } {
  let verifiedUntil = confirmed ? Date.now() + 5 * 60_000 : 0
  const host: VerificationHost = {
    verifyPassword: async (password) => {
      await new Promise((resolve) => setTimeout(resolve, 500))
      if (password === 'wrong') {
        return { kind: 'wrongPassword', message: 'That is not your password.' }
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
  }
  return { host, verified: () => Date.now() < verifiedUntil }
}

/** How a specimen's dialog opens: on the confirmation, on a code, on an expired code, or handed off to the phone. */
export type HandoffSpecimenStart = 'verify' | 'code' | 'expired' | 'handedOff'

/** The browser of the stand-in phone the code is handed off to. */
const PHONE = 'Safari on iPhone'

/**
 * A host of Hand Off to Your Phone as the product's would answer, for one
 * specimen: it confirms as `galleryVerification` does; a code lasts ten
 * minutes, and the confirmation five, so asking again within them shows a
 * code at once. `phoneScans` stands in for the phone's camera.
 */
export function galleryHandoff(start: HandoffSpecimenStart): { host: HandoffHost; phoneScans: () => void } {
  // The pinned code states start confirmed.
  const verification = galleryVerification(start !== 'verify')
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
      if (!verification.verified()) {
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
    ...verification.host,
    handedOffTo: (id) => (handedOffCode.value === id ? handedOff.value : null),
  }

  function phoneScans(): void {
    handedOffCode.value = `handoff-${codes}`
    handedOff.value = PHONE
  }

  return { host, phoneScans }
}

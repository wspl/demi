import type { PasskeyOfferOutcome } from '@demicodes/web-ui/settings/types'
import { productWould } from '../product-would'

/** How the stand-in browser answers Add Passkey in a specimen of the offer. */
export type PasskeyOfferScript = 'adds' | 'needsPassword' | 'held' | 'fails'

/**
 * A host of the passkey offer as the product's would answer, for one
 * specimen: `adds` saves the passkey; `needsPassword` asks for the password
 * first, as five minutes after the sign-in, and refuses `wrong`; `held` is a
 * password manager that has a passkey of the account already; `fails` is a
 * backend that cannot verify it.
 */
export function galleryPasskeyOffer(script: PasskeyOfferScript): (password: string | null) => Promise<PasskeyOfferOutcome> {
  return async (password) => {
    await new Promise((resolve) => setTimeout(resolve, 500))
    switch (script) {
      case 'needsPassword':
        if (password === null) {
          return { kind: 'needsPassword' }
        }
        if (password === 'wrong') {
          return { kind: 'wrongPassword', message: 'That is not your password.' }
        }
        break
      case 'held':
        return { kind: 'held', message: 'This password manager has a passkey for your account already.' }
      case 'fails':
        return { kind: 'failed', message: 'The passkey could not be verified', requestId: 'req_gallery0000000001' }
      case 'adds':
        break
    }
    productWould('Ask the Browser to Save a Passkey')
    return { kind: 'added' }
  }
}

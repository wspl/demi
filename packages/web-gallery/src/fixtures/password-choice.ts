import type { PasswordChoiceOutcome } from '@demicodes/web-ui/settings/types'

/** How the host answers Choose Password in a specimen: the choice is saved, or the backend refuses it. */
export type PasswordChoiceScript = 'saves' | 'fails'

/**
 * A host of Choose Your Password as the product's would answer, for one
 * specimen, after a beat, as a request to the backend would.
 */
export function galleryPasswordChoice(script: PasswordChoiceScript): (password: string) => Promise<PasswordChoiceOutcome> {
  return async () => {
    await new Promise((resolve) => setTimeout(resolve, 500))
    return script === 'fails'
      ? { kind: 'failed', message: 'The password could not be saved', requestId: 'req_gallery0000000002' }
      : { kind: 'chosen' }
  }
}

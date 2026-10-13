<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { reportError, shownError } from '@demicodes/web-ui/infra/errors'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ChoosePasswordWizard from '@demicodes/web-ui/auth/ChoosePasswordWizard.vue'
import type { PasswordChoiceOutcome } from '@demicodes/web-ui/settings/types'
import { readLocalState, writeLocalState } from '../state/local'
import { addPasskey } from './passkeys'
import { usePasskeyOffer } from './passkey-offer'
import { setupStatus, useSession } from './session'
import { browserHasPasskeys } from './webauthn'

/**
 * Choose Your Password and the passkey offer after it (`product.md` § User
 * system): the wizard an account signed in with a first password sees
 * before the product, over a blank page. Choosing it signs the account's
 * other pages up for the same, and the offer, where this server and browser
 * have passkeys, follows in the same dialog; once it ends, the product opens.
 */
const session = useSession()
const offered = usePasskeyOffer()
/** Whether the passkey offer follows the choice, once the server says it has passkeys. */
const offer = ref(false)
/** Whether the offer was answered, which this browser remembers for the account. */
let answered = false

onMounted(async () => {
  try {
    offer.value = browserHasPasskeys() && (await setupStatus()).passkeyRpId != null
  } catch {
    // A server that does not say offers none.
    offer.value = false
  }
})

async function choose(password: string): Promise<PasswordChoiceOutcome> {
  try {
    await session.choosePassword(password)
    return { kind: 'chosen' }
  } catch (error) {
    return { kind: 'failed', ...shownError(error) }
  }
}

function markAnswered(): void {
  answered = true
}

function finish(): void {
  const user = session.current.status === 'choosingPassword' ? session.current.user : null
  if (user && answered) {
    writeLocalState(user.id, { ...readLocalState(user.id), passkeyOfferAnswered: true })
  }
  // This wizard was the offer a handoff asked for, if one did.
  offered.close()
  session.finishChoosing()
}

async function signOut(): Promise<void> {
  try {
    await session.signOut()
    // A new document releases account-scoped stores, sockets and draft data.
    window.location.replace('/login')
  } catch (error) {
    reportError('Could Not Sign Out', error, { userVisible: true })
  }
}
</script>

<template>
  <ChoosePasswordWizard
    v-if="session.current.status === 'choosingPassword'"
    is-open
    :overlay-store="appOverlayStore"
    :email="session.current.user.email"
    :choose="choose"
    :offer="offer"
    :add="addPasskey"
    @answered="markAnswered"
    @done="finish"
    @sign-out="signOut"
  />
</template>

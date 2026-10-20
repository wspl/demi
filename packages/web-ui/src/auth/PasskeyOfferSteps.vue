<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { ref, watch } from 'vue'
import { KeyRound } from '@lucide/vue'
import type { ShownError } from '../infra/errors'
import type { PasskeyOfferOutcome, PasskeyOfferStep } from '../settings/types'
import Button from '../ui/Button.vue'
import { hiddenUsername } from '../ui/field-autofill'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { deviceSystem, unlockSentence, useDeviceReport } from './device-unlock'
import WizardStep from '../ui/WizardStep.vue'
import { useOpening } from '../ui/operations'

/**
 * The steps of the offer to add a passkey, which a wizard frames: the offer
 * alone (`PasskeyOffer`) or after the password a first sign-in chose
 * (`ChoosePasswordWizard`). Each opening starts over.
 *
 * The offer to add a passkey, *Use a passkey next time?* with Add Passkey and
 * Not Now: a one-step wizard, which a host opens once on a browser after a
 * sign-in that has just proved the user: a handoff code's, a password chosen at
 * the first sign-in. It is a wizard of one step so the flows that end in it
 * hold it as one: Choose Your Password goes on into it in the same dialog, and
 * on a phone it is the whole screen.
 *
 * Add Passkey asks the host to add one without the password, which a sign-in
 * of the last five minutes has already proved; once those have passed the
 * host answers it needs the password, and the step asks for it. A password
 * manager that holds a passkey of the account says so, in the step
 * *Passkey Ready*: not a failure. A dismissed browser prompt leaves the step as
 * it was, for another try; a failure is said in the step. Not Now, Escape and
 * the scrim leave it.
 *
 * `answered` is emitted once, when Add Passkey or a way out is first used,
 * for the host to remember that the browser was asked; `close` when the
 * wizard is done. The sentence under the title names how this device unlocks,
 * from the browser's system (`device-unlock.ts`): Face ID or Touch ID on an
 * iPhone or iPad, Touch ID on a Mac, Windows Hello, an Android's fingerprint,
 * face or screen lock, else the screen lock; the symbol is a key on every
 * device. The browser's own prompt then names its method.
 */
const props = defineProps<{
  /** Whether the wizard is open: each opening starts over, as a new browser's would. */
  isOpen: boolean
  add: (password: string | null) => Promise<PasskeyOfferOutcome>
  /** The account's email, which the password manager files the password under. */
  email: string
}>()

const emit = defineEmits<{
  answered: []
  close: []
}>()

// What the device unlocks with, from the browser's system: said in the first step.
const unlock = unlockSentence(deviceSystem(useDeviceReport()))

const step = defineModel<PasskeyOfferStep>('step', { default: 'ask' })
const password = ref('')
/** What is wrong with the password, said under it; null when nothing is. */
const passwordProblem = ref<string | null>(null)
const error = ref<ShownError | null>(null)
/** Continue, whose press Enter in the field stands for, so a second one meanwhile does nothing. */
const continueButton = ref<InstanceType<typeof Button> | null>(null)
let answered = false

// Each opening starts over, as a new browser's would.
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      step.value = 'ask'
      password.value = ''
      passwordProblem.value = null
      error.value = null
      answered = false
    }
  },
)
watch(password, () => {
  passwordProblem.value = null
})

function answer(): void {
  if (!answered) {
    answered = true
    emit('answered')
  }
}

const opening = useOpening(() => props.isOpen)
/** Add Passkey and Continue are one operation, whichever step shows it. */
const adding = { action: 'add', target: 'passkey:offer' }

function notNow(): void {
  answer()
  emit('close')
}

/** Add Passkey or Continue: the button works until the host and the browser's prompt have answered. */
async function addPasskey(): Promise<void> {
  if (step.value === 'password' && password.value === '') {
    passwordProblem.value = t('password.required')
    return
  }
  answer()
  error.value = null
  const stillShown = opening.shownNow()
  const outcome = await props.add(step.value === 'password' ? password.value : null)
  // Its answer is for the wizard it was asked in, not one opened since.
  if (!stillShown()) {
    return
  }
  switch (outcome.kind) {
    case 'added':
      emit('close')
      break
    case 'held':
      step.value = 'held'
      break
    case 'needsPassword':
      step.value = 'password'
      break
    case 'wrongPassword':
      passwordProblem.value = outcome.message
      break
    case 'dismissed':
      break
    case 'failed':
      error.value = outcome
      break
  }
}
</script>

<template>
  <WizardStep
    v-if="step === 'ask'"
    :title="t('offer.title')"
    :icon="KeyRound"
    :message="unlock"
  >
    <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
    <template #actions>
      <Button @click="notNow">{{ t('offer.notNow') }}</Button>
      <Button variant="primary" :operation="adding" @click="addPasskey">{{ t('offer.add') }}</Button>
    </template>
  </WizardStep>
  <WizardStep
    v-else-if="step === 'password'"
    :title="t('offer.passwordTitle')"
    :icon="KeyRound"
    :message="t('offer.passwordMessage')"
  >
    <form class="flex flex-col gap-2" @submit.prevent="continueButton?.press()">
      <TextInput
        v-model="password"
        :aria-label="t('offer.password')"
        masked="password"
        focused
        autocomplete="current-password"
        :invalid="passwordProblem !== null"
      />
      <InlineError v-if="passwordProblem" :message="passwordProblem" />
      <input v-bind="hiddenUsername(email)" />
    </form>
    <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
    <template #actions>
      <Button @click="notNow">{{ t('offer.notNow') }}</Button>
      <Button ref="continueButton" variant="primary" :operation="adding" @click="addPasskey">{{ uiText('common.continue') }}</Button>
    </template>
  </WizardStep>
  <WizardStep
    v-else
    :title="t('offer.readyTitle')"
    :message="t('offer.readyMessage')"
    :icon="KeyRound"
  >
    <template #actions>
      <Button variant="primary" @click="emit('close')">{{ uiText('common.continue') }}</Button>
    </template>
  </WizardStep>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import type { ShownError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import type { PasskeyOfferOutcome } from '../settings/types'
import Button from '../ui/Button.vue'
import { hiddenUsername } from '../ui/field-autofill'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import Wizard from '../ui/Wizard.vue'
import WizardStep from '../ui/WizardStep.vue'

/**
 * The offer to add a passkey (`product.md` § User system), *Use Face ID next
 * time?* with Add Passkey and Not Now: a one-step wizard
 * (`web-application.md` § Responding to the user), which a host opens once
 * on a browser after a sign-in that has just proved the user: a handoff
 * code's, a password chosen at the first sign-in. It is a wizard of one
 * step so the flows that end in it hold it as one: Choose Your Password goes
 * on into it in the same dialog, and on a phone it is the whole screen.
 *
 * Add Passkey asks the host to add one without the password, which a sign-in
 * of the last five minutes has already proved; once those have passed the
 * host answers it needs the password, and the step asks for it. A password
 * manager that holds a passkey of the account says so, and the step
 * says it too: not a failure. A dismissed browser prompt leaves the step as
 * it was, for another try; a failure is said in the step. Not Now, Escape and
 * the scrim leave it.
 *
 * `answered` is emitted once, when Add Passkey or a way out is first used,
 * for the host to remember that the browser was asked; `close` when the
 * wizard is done. It does not know the browser's wording: on a device without
 * Face ID the browser's own prompt names its method.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  add: (password: string | null) => Promise<PasskeyOfferOutcome>
  /** The account's email, which the password manager files the password under. */
  email: string
}>()

const emit = defineEmits<{
  answered: []
  close: []
}>()

type Step = 'ask' | 'password' | 'held'
const step = ref<Step>('ask')
const password = ref('')
/** What is wrong with the password, said under it; null when nothing is. */
const passwordProblem = ref<string | null>(null)
const error = ref<ShownError | null>(null)
const busy = ref(false)
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

function notNow(): void {
  if (busy.value) {
    return
  }
  answer()
  emit('close')
}

async function addPasskey(): Promise<void> {
  if (busy.value) {
    return
  }
  if (step.value === 'password' && password.value === '') {
    passwordProblem.value = 'Required'
    return
  }
  answer()
  busy.value = true
  error.value = null
  try {
    const outcome = await props.add(step.value === 'password' ? password.value : null)
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
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <Wizard
    :is-open="isOpen"
    :overlay-store="overlayStore"
    label="Add a passkey"
    :step="step"
    skippable
    @close="notNow"
  >
    <WizardStep
      v-if="step === 'ask'"
      title="Use Face ID next time?"
      message="A passkey signs you in to Demi without your password."
    >
      <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
      <template #actions>
        <Button :disabled="busy" @click="notNow">Not Now</Button>
        <Button variant="primary" :loading="busy" @click="addPasskey">Add Passkey</Button>
      </template>
    </WizardStep>
    <WizardStep
      v-else-if="step === 'password'"
      title="Enter Your Password"
      message="Confirm it is you to add the passkey."
    >
      <form class="flex flex-col gap-2" @submit.prevent="addPasskey">
        <TextInput
          v-model="password"
          aria-label="Password"
          masked="password"
          focused
          autocomplete="current-password"
          :invalid="passwordProblem !== null"
          :readonly="busy"
        />
        <InlineError v-if="passwordProblem" :message="passwordProblem" />
        <input v-bind="hiddenUsername(email)" />
      </form>
      <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
      <template #actions>
        <Button :disabled="busy" @click="notNow">Not Now</Button>
        <Button variant="primary" :loading="busy" @click="addPasskey">Continue</Button>
      </template>
    </WizardStep>
    <WizardStep v-else title="This phone already has a passkey for Demi.">
      <template #actions>
        <Button variant="primary" @click="emit('close')">OK</Button>
      </template>
    </WizardStep>
  </Wizard>
</template>

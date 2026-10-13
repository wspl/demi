<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { ShownError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import type { PasskeyOfferOutcome, PasskeyOfferStep, PasswordChoiceOutcome } from '../settings/types'
import Button from '../ui/Button.vue'
import { hiddenUsername } from '../ui/field-autofill'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import Wizard from '../ui/Wizard.vue'
import WizardStep from '../ui/WizardStep.vue'
import PasskeyOfferSteps from './PasskeyOfferSteps.vue'
import {
  PASSWORD_HINT,
  PASSWORD_MIN_LENGTH,
  PASSWORD_TOO_SHORT,
  PASSWORDS_DIFFER,
  differs,
  isTooShort,
} from './password'

/**
 * Choose Your Password and the passkey offer after it, one wizard
 * (`product.md` § User system, `web-application.md` § Responding to the
 * user): what an account an administrator made or reset sees before
 * anything else of the product opens, on a computer one dialog at the centre
 * over the page, on a phone the whole screen. The password it was given is
 * only a first one: the first step asks for its own, typed twice, without the
 * current one, and has no close button, since it cannot be skipped; Sign Out
 * is the one other way out. When the host offers passkeys, the steps of
 * the offer follow in the same dialog, with Add Passkey and Not Now, which
 * leave it. Length and the match are checked here; the host's answer to the
 * choice is `choose`'s outcome, a failure said in the step.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The account's email, which the password manager files the new password under. */
  email: string
  choose: (password: string) => Promise<PasswordChoiceOutcome>
  /** Whether the passkey offer follows the choice. */
  offer: boolean
  add: (password: string | null) => Promise<PasskeyOfferOutcome>
}>()

const emit = defineEmits<{
  /** The offer was answered, by Add Passkey or a way out: this browser was asked. */
  answered: []
  /** The wizard is over: the account may go on into the product. */
  done: []
  signOut: []
}>()

const step = ref<'choose' | PasskeyOfferStep>('choose')
const offerStep = computed<PasskeyOfferStep>({
  get: () => (step.value === 'choose' ? 'ask' : step.value),
  set: (value) => {
    step.value = value
  },
})
const password = ref('')
const confirmation = ref('')
const error = ref<ShownError | null>(null)
const busy = ref(false)

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      step.value = 'choose'
      password.value = ''
      confirmation.value = ''
      error.value = null
    }
  },
)

const tooShort = computed(() => isTooShort(password.value))
const mismatch = computed(() => differs(password.value, confirmation.value))
const canSubmit = computed(
  () => !busy.value && password.value.length >= PASSWORD_MIN_LENGTH && confirmation.value === password.value,
)

async function submit(): Promise<void> {
  if (!canSubmit.value) {
    return
  }
  busy.value = true
  error.value = null
  try {
    const outcome = await props.choose(password.value)
    if (outcome.kind === 'failed') {
      error.value = outcome
      return
    }
    // The password is chosen and forgotten; the offer, when there is one, goes on in place.
    password.value = ''
    confirmation.value = ''
    if (props.offer) {
      step.value = 'ask'
    } else {
      emit('done')
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
    label="Choose Your Password"
    :step="step === 'choose' ? 'choose' : 'offer'"
    :skippable="step !== 'choose'"
    @close="emit('done')"
  >
    <WizardStep
      v-if="step === 'choose'"
      title="Choose Your Password"
      message="The password you were given is only a first one. Choose your own to go on."
    >
      <form class="flex flex-col gap-4" @submit.prevent="submit">
        <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
          New password
          <TextInput
            v-model="password"
            masked="password"
            autocomplete="new-password"
            name="password"
            focused
            :disabled="busy"
          />
          <span class="text-[12px] leading-4 text-fg-subtle">{{ PASSWORD_HINT }}</span>
        </label>
        <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
          Confirm new password
          <TextInput
            v-model="confirmation"
            masked="password"
            autocomplete="new-password"
            name="confirmation"
            :disabled="busy"
          />
        </label>
        <input v-bind="hiddenUsername(email)" />
        <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
        <InlineError v-else-if="tooShort" :message="PASSWORD_TOO_SHORT" />
        <InlineError v-else-if="mismatch" :message="PASSWORDS_DIFFER" />
      </form>
      <template #actions>
        <Button :disabled="busy" @click="emit('signOut')">Sign Out</Button>
        <Button variant="primary" :disabled="!canSubmit && !busy" :loading="busy" @click="submit">Choose Password</Button>
      </template>
    </WizardStep>
    <PasskeyOfferSteps
      v-else
      v-model:step="offerStep"
      :is-open="isOpen"
      :add="add"
      :email="email"
      @answered="emit('answered')"
      @close="emit('done')"
    />
  </Wizard>
</template>

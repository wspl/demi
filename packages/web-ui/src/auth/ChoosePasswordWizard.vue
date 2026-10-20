<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import { Lock } from '@lucide/vue'
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
  PASSWORD_MIN_LENGTH,
  passwordHint,
  passwordTooShort,
  passwordsDiffer,
  differs,
  isTooShort,
} from './password'

/**
 * Choose Your Password and the passkey offer after it, one wizard: what an
 * account an administrator made or reset sees before anything else of the
 * product opens, on a computer one dialog at the centre over the page, on a
 * phone the whole screen. The password it was given is only a first one: the
 * first step asks for its own, typed twice, without the current one, and has no
 * close button, since it cannot be skipped; Sign Out is the one other way out.
 * When the host offers passkeys, the steps of the offer follow in the same
 * dialog, with Add Passkey and Not Now, which leave it. Length and the match
 * are checked here; the host's answer to the choice is `choose`'s outcome, a
 * failure said in the step.
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
/** Choose Password, whose press Enter in a field stands for, so a second one meanwhile does nothing. */
const chooseButton = ref<InstanceType<typeof Button> | null>(null)

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
  () => password.value.length >= PASSWORD_MIN_LENGTH && confirmation.value === password.value,
)

/** Choose Password: the button works until the host has answered. */
async function submit(): Promise<void> {
  if (!canSubmit.value) {
    return
  }
  error.value = null
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
}
</script>

<template>
  <Wizard
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="t('choose.title')"
    :step="step === 'choose' ? 'choose' : 'offer'"
    :skippable="step !== 'choose'"
    @close="emit('done')"
  >
    <WizardStep
      v-if="step === 'choose'"
      :title="t('choose.title')"
      :icon="Lock"
      :message="t('choose.message')"
    >
      <form class="flex flex-col gap-4" @submit.prevent="chooseButton?.press()">
        <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
          {{ t('choose.new') }}
          <TextInput
            v-model="password"
            masked="password"
            autocomplete="new-password"
            name="password"
            focused
          />
          <span class="text-detail leading-4 text-fg-subtle">{{ passwordHint() }}</span>
        </label>
        <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
          {{ t('choose.confirm') }}
          <TextInput
            v-model="confirmation"
            masked="password"
            autocomplete="new-password"
            name="confirmation"
          />
        </label>
        <input v-bind="hiddenUsername(email)" />
        <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
        <InlineError v-else-if="tooShort" :message="passwordTooShort()" />
        <InlineError v-else-if="mismatch" :message="passwordsDiffer()" />
      </form>
      <template #actions>
        <Button @click="emit('signOut')">{{ t('choose.signOut') }}</Button>
        <Button
          ref="chooseButton"
          variant="primary"
          :disabled="!canSubmit"
          :operation="{ action: 'choose', target: 'account:password' }"
          @click="submit"
        >
          {{ t('choose.choose') }}
        </Button>
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

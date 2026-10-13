<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import { ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { hiddenUsername } from '../ui/field-autofill'
import SettingsRow from './SettingsRow.vue'
import type { PasskeyAddOutcome } from './types'

/**
 * Adding a passkey: the account's password first, so a browser left signed
 * in cannot add one, then the browser's own prompt, which the host opens and
 * the password manager answers. An empty password is refused under its
 * field on Continue, a wrong one too, in the host's words; the field turns
 * red until it changes. A dismissed prompt leaves the dialog as it was, for
 * another try; an added passkey closes it, and the list shows it.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The account's email, which the password manager files the password under. */
  email: string
  add: (password: string) => Promise<PasskeyAddOutcome>
}>()

const emit = defineEmits<{
  close: []
}>()

const password = ref('')
/** What is wrong with the password, said under it; null when nothing is. */
const passwordProblem = ref<string | null>(null)
const error = ref<ShownError | null>(null)
/** Continue, whose press Enter in the field stands for, so a second one meanwhile does nothing. */
const continueButton = ref<InstanceType<typeof Button> | null>(null)

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      password.value = ''
      passwordProblem.value = null
      error.value = null
    }
  },
)
watch(password, () => {
  passwordProblem.value = null
})

/** Continue: the button works until the host and the browser's prompt have answered. */
async function submit(): Promise<void> {
  if (password.value === '') {
    passwordProblem.value = 'Required'
    return
  }
  error.value = null
  const outcome = await props.add(password.value)
  switch (outcome.kind) {
    case 'added':
      emit('close')
      break
    case 'wrongPassword':
      passwordProblem.value = outcome.message
      break
    case 'dismissed':
      break
    case 'held':
      error.value = { message: outcome.message, requestId: null }
      break
    case 'failed':
      error.value = outcome
      break
  }
}
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    label="Add Passkey"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add Passkey</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          Enter your password. Your browser then saves a passkey in its password manager, and signs you in with it from
          then on.
        </p>
      </header>
      <form
        class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        @submit.prevent="continueButton?.press()"
      >
        <SettingsRow label="Password">
          <template v-if="passwordProblem" #detail>
            <InlineError :message="passwordProblem" />
          </template>
          <TextInput
            v-model="password"
            aria-label="Password"
            masked="password"
            focused
            autocomplete="current-password"
            :invalid="passwordProblem !== null"
            class="w-48 max-w-full"
          />
        </SettingsRow>
        <!-- Last in the card, so the card's row dividers skip it; it is out of the flow. -->
        <input v-bind="hiddenUsername(email)" />
      </form>
      <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
    </div>
    <template #footer>
      <Button @click="emit('close')">Cancel</Button>
      <Button ref="continueButton" variant="primary" @click="submit">Continue</Button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import { ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { hiddenUsername } from '../ui/field-autofill'
import { useOpening } from '../ui/operations'
import SettingsRow from './SettingsRow.vue'
import type { PasskeyAddOutcome } from './types'

/**
 * Adding a passkey: the account's password first, so a browser left signed in
 * cannot add one, then the browser's own prompt, which the host opens and the
 * password manager answers.
 *
 * - An empty password is refused under its field on Continue, a wrong one too, in
 *   the host's words. The field turns red until it changes.
 * - A dismissed prompt leaves the dialog as it was, for another try.
 * - An added passkey closes the dialog, and the list shows it.
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

const opening = useOpening(() => props.isOpen)

/** Continue: the button works until the host and the browser's prompt have answered, which this opening alone shows. */
async function submit(): Promise<void> {
  if (password.value === '') {
    passwordProblem.value = t('addPasskey.required')
    return
  }
  error.value = null
  const stillShown = opening.shownNow()
  const outcome = await props.add(password.value)
  if (!stillShown()) {
    return
  }
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
    :label="t('addPasskey.title')"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('addPasskey.title') }}</h3>
        <p class="mt-0.5 text-explanation leading-5 text-fg-muted">
          {{ t('addPasskey.hint') }}
        </p>
      </header>
      <form
        class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        @submit.prevent="continueButton?.press()"
      >
        <SettingsRow :label="t('addPasskey.password')">
          <template v-if="passwordProblem" #detail>
            <InlineError :message="passwordProblem" />
          </template>
          <TextInput
            v-model="password"
            :aria-label="t('addPasskey.password')"
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
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button ref="continueButton" variant="primary" :operation="{ action: 'add', target: 'passkey:new' }" @click="submit">
        {{ uiText('common.continue') }}
      </Button>
    </template>
  </Dialog>
</template>

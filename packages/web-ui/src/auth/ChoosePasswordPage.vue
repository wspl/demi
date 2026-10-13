<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import { computed } from 'vue'
import Button from '../ui/Button.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { hiddenUsername } from '../ui/field-autofill'
import AuthPage from './AuthPage.vue'
import {
  PASSWORD_HINT,
  PASSWORD_MIN_LENGTH,
  PASSWORD_TOO_SHORT,
  PASSWORDS_DIFFER,
  differs,
  isTooShort,
} from './password'

/**
 * The page an account sees at its first sign-in, and again after an
 * administrator reset its password (`product.md` § User system): the
 * password it was given is only a first one, so before anything else it
 * chooses its own, typed twice, without the current one. Nothing else of the
 * product opens until it is chosen; Sign Out is the one other way out. Length
 * and the match are checked here; the host reports busy and a failure.
 */
const props = defineProps<{
  /** The account's email, which the password manager files the new password under. */
  email: string
  busy?: boolean
  error?: ShownError
}>()

const password = defineModel<string>('password', { default: '' })
const confirmation = defineModel<string>('confirmation', { default: '' })

const emit = defineEmits<{
  submit: [password: string]
  signOut: []
}>()

const tooShort = computed(() => isTooShort(password.value))
const mismatch = computed(() => differs(password.value, confirmation.value))
const canSubmit = computed(
  () =>
    !props.busy &&
    password.value.length >= PASSWORD_MIN_LENGTH &&
    confirmation.value === password.value,
)

function submit() {
  if (canSubmit.value) {
    emit('submit', password.value)
  }
}
</script>

<template>
  <AuthPage
    title="Choose Your Password"
    message="The password you were given is only a first one. Choose your own to go on."
  >
    <form class="flex flex-col gap-4" @submit.prevent="submit">
      <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
        New password
        <TextInput
          v-model="password"
          size="lg"
          masked="password"
          autocomplete="new-password"
          name="password"
          :disabled="busy"
          focused
          @keydown.enter="submit"
        />
        <span class="text-[12px] leading-4 text-fg-subtle">{{ PASSWORD_HINT }}</span>
      </label>
      <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
        Confirm new password
        <TextInput
          v-model="confirmation"
          size="lg"
          masked="password"
          autocomplete="new-password"
          name="confirmation"
          :disabled="busy"
          @keydown.enter="submit"
        />
      </label>
      <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
      <InlineError v-else-if="tooShort" :message="PASSWORD_TOO_SHORT" />
      <InlineError v-else-if="mismatch" :message="PASSWORDS_DIFFER" />
      <Button
        size="lg"
        variant="primary"
        class="mt-1 w-full"
        :disabled="!canSubmit && !busy"
        :loading="busy"
        @click="submit"
      >
        Choose Password
      </Button>
      <Button variant="ghost" class="w-full" :disabled="busy" @click="emit('signOut')">Sign Out</Button>
      <!-- Last, out of the flow, so the password manager files the new password under the account. -->
      <input v-bind="hiddenUsername(email)" />
    </form>
  </AuthPage>
</template>

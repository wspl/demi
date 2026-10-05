<script setup lang="ts">
import { computed } from 'vue'
import Button from '../ui/Button.vue'
import AuthPage from './AuthPage.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { isEmail } from './email'

/**
 * Email and password on the auth page. Owns nothing; the host reports busy,
 * error, and why the user is here.
 */
export type EmailLoginPhase = {
  busy?: boolean
  error?: string
  /** The session ended and sent them back to this page. */
  reason?: 'expired'
}

const props = defineProps<{
  phase: EmailLoginPhase
}>()

const email = defineModel<string>('email', { default: '' })
const password = defineModel<string>('password', { default: '' })

const emit = defineEmits<{
  submit: [email: string, password: string]
}>()

const canSubmit = computed(
  () => !props.phase.busy && isEmail(email.value) && password.value.length > 0,
)

function submit() {
  if (!canSubmit.value) return
  emit('submit', email.value.trim(), password.value)
}
</script>

<template>
  <AuthPage
    title="Sign In"
    :message="
      phase.reason === 'expired'
        ? 'Your session ended. Sign in again.'
        : 'Sign in with your email.'
    "
  >
    <form class="flex flex-col gap-4" @submit.prevent="submit">
      <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
        Email
        <TextInput
          v-model="email"
          size="lg"
          autocomplete="email"
          inputmode="email"
          name="email"
          :disabled="phase.busy"
          focused
          @keydown.enter="submit"
        />
      </label>
      <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
        Password
        <TextInput
          v-model="password"
          size="lg"
          secret
          autocomplete="current-password"
          name="password"
          :disabled="phase.busy"
          @keydown.enter="submit"
        />
      </label>
      <InlineError v-if="phase.error" :message="phase.error" />
      <Button
        size="lg"
        variant="primary"
        class="mt-1 w-full"
        :disabled="!canSubmit && !phase.busy"
        :loading="phase.busy"
        @click="submit"
      >
        Sign In
      </Button>
    </form>
  </AuthPage>
</template>

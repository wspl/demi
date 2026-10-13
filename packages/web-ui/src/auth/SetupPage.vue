<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import { computed } from 'vue'
import Button from '../ui/Button.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import AuthPage from './AuthPage.vue'
import { isEmail } from './email'
import {
  PASSWORD_HINT,
  PASSWORD_MIN_LENGTH,
  PASSWORD_TOO_SHORT,
  PASSWORDS_DIFFER,
  differs,
  isTooShort,
} from './password'

/**
 * The first visitor's page on a Demi without accounts: it creates the master
 * account, which manages Demi and every account added later. Owns nothing;
 * the host reports busy, an error, and whether another visitor set Demi up
 * first. Once the account is made, where the server and the browser offer
 * passkeys, the host shows the optional Add a Passkey step: Add Passkey
 * asks the browser for one with the password just entered, and Skip goes on
 * without.
 */
export type SetupPhase = {
  busy?: boolean
  error?: ShownError
  /** The master account exists already: only signing in is left. */
  taken?: boolean
  /** The account is made and signed in: the optional passkey step. */
  passkey?: {
    busy?: boolean
    error?: ShownError
  }
}

const props = defineProps<{
  phase: SetupPhase
}>()

/** The most characters a name has once trimmed (`web-api.md` § Account API). */
const NAME_MAX_LENGTH = 80

const name = defineModel<string>('name', { default: '' })
const email = defineModel<string>('email', { default: '' })
const password = defineModel<string>('password', { default: '' })
const confirmation = defineModel<string>('confirmation', { default: '' })

const emit = defineEmits<{
  submit: [name: string, email: string, password: string]
  signIn: []
  addPasskey: []
  skipPasskey: []
}>()

const nameLength = computed(() => Array.from(name.value.trim()).length)
const tooShort = computed(() => isTooShort(password.value))
const mismatch = computed(() => differs(password.value, confirmation.value))
const canSubmit = computed(
  () =>
    !props.phase.busy &&
    nameLength.value >= 1 &&
    nameLength.value <= NAME_MAX_LENGTH &&
    isEmail(email.value) &&
    password.value.length >= PASSWORD_MIN_LENGTH &&
    confirmation.value === password.value,
)

function submit() {
  if (!canSubmit.value) return
  emit('submit', name.value.trim(), email.value.trim(), password.value)
}
</script>

<template>
  <AuthPage
    v-if="phase.passkey"
    title="Add a Passkey"
    message="Sign in next time with Touch ID, Face ID or your password manager, without your password. You can add one later in Settings › Account."
  >
    <div class="flex flex-col gap-4">
      <!-- The line keeps its height with nothing to say, so a message never moves the buttons. -->
      <div class="min-h-4">
        <InlineError v-if="phase.passkey.error" :message="phase.passkey.error.message" :request-id="phase.passkey.error.requestId" />
      </div>
      <Button
        size="lg"
        variant="primary"
        class="w-full"
        :loading="phase.passkey.busy"
        @click="emit('addPasskey')"
      >
        Add Passkey
      </Button>
      <Button size="lg" class="w-full" :disabled="phase.passkey.busy" @click="emit('skipPasskey')">Skip</Button>
    </div>
  </AuthPage>
  <AuthPage
    v-else-if="phase.taken"
    title="Set Up Demi"
    message="Demi is set up already. Sign in with its master account."
  >
    <Button
      size="lg"
      variant="primary"
      class="w-full"
      @click="emit('signIn')"
    >
      Sign In
    </Button>
  </AuthPage>
  <AuthPage
    v-else
    title="Set Up Demi"
    message="Create the master account. It manages Demi and every account you add."
  >
    <form class="flex flex-col gap-4" @submit.prevent="submit">
      <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
        Name
        <TextInput
          v-model="name"
          size="lg"
          autocomplete="name"
          name="name"
          :disabled="phase.busy"
          focused
          @keydown.enter="submit"
        />
      </label>
      <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
        Email
        <TextInput
          v-model="email"
          size="lg"
          autocomplete="email"
          inputmode="email"
          name="email"
          :disabled="phase.busy"
          @keydown.enter="submit"
        />
      </label>
      <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
        Password
        <TextInput
          v-model="password"
          size="lg"
          masked="password"
          autocomplete="new-password"
          name="password"
          :disabled="phase.busy"
          @keydown.enter="submit"
        />
        <span class="text-[12px] leading-4 text-fg-subtle">{{
          PASSWORD_HINT
        }}</span>
      </label>
      <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
        Confirm password
        <TextInput
          v-model="confirmation"
          size="lg"
          masked="password"
          autocomplete="new-password"
          name="confirmation"
          :disabled="phase.busy"
          @keydown.enter="submit"
        />
      </label>
      <InlineError v-if="phase.error" :message="phase.error.message" :request-id="phase.error.requestId" />
      <InlineError v-else-if="tooShort" :message="PASSWORD_TOO_SHORT" />
      <InlineError v-else-if="mismatch" :message="PASSWORDS_DIFFER" />
      <Button
        size="lg"
        variant="primary"
        class="mt-1 w-full"
        :disabled="!canSubmit && !phase.busy"
        :loading="phase.busy"
        @click="submit"
      >
        Create Account
      </Button>
    </form>
  </AuthPage>
</template>

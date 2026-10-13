<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import Button from '../ui/Button.vue'
import AuthPage from './AuthPage.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { isEmail } from './email'

/**
 * Email and password on the auth page, and passkeys where the host offers
 * them. Owns nothing; the host reports busy, error, a lockout and why the
 * user is here.
 *
 * While a sign-in runs the fields keep the focus, read-only. A wrong password
 * gives the focus back to the password field with its text selected, as
 * macOS does, so the next try is typed at once; the line under the fields
 * keeps its height either way, so nothing moves. A lockout counts down to its
 * end and keeps Sign In off until then.
 *
 * With `passkeys`, the email field offers the browser's passkeys in its
 * autofill (`autocomplete="username webauthn"`, which the host's request
 * with conditional mediation fills), and Sign In with Passkey asks the
 * browser for one; neither needs an email first. A passkey's refusal is the
 * same line under the fields, and leaves the focus where it is.
 */
export type EmailLoginPhase = {
  busy?: boolean
  error?: ShownError
  /** The address is locked until this time, in milliseconds since the epoch. */
  lockedUntil?: number
  /** The session ended and sent them back to this page. */
  reason?: 'expired'
  /** A passkey sign-in runs. */
  passkeyBusy?: boolean
  /** Why the last passkey sign-in failed. */
  passkeyError?: ShownError
}

const props = defineProps<{
  phase: EmailLoginPhase
  /** The server and the browser offer passkeys. */
  passkeys?: boolean
}>()

const email = defineModel<string>('email', { default: '' })
const password = defineModel<string>('password', { default: '' })

const emit = defineEmits<{
  submit: [email: string, password: string]
  /** Sign In with Passkey. */
  passkey: []
}>()

const passwordField = ref<InstanceType<typeof TextInput>>()

/** The time the countdown reads, moved each second while a lock holds. */
const now = ref(Date.now())
let ticking: ReturnType<typeof setInterval> | null = null

function stopTicking(): void {
  if (ticking !== null) {
    clearInterval(ticking)
    ticking = null
  }
}

watch(
  () => props.phase.lockedUntil,
  (until) => {
    stopTicking()
    now.value = Date.now()
    if (until === undefined || until <= now.value) {
      return
    }
    ticking = setInterval(() => {
      now.value = Date.now()
      if (now.value >= until) {
        stopTicking()
      }
    }, 1000)
  },
  { immediate: true },
)
onBeforeUnmount(stopTicking)

/** The seconds the lock still holds; 0 when there is none. */
const lockedSeconds = computed(() =>
  props.phase.lockedUntil === undefined
    ? 0
    : Math.max(0, Math.ceil((props.phase.lockedUntil - now.value) / 1000)),
)

const countdown = computed(() => {
  const seconds = lockedSeconds.value
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
})

/** The line under the fields: the countdown while locked, else the host's error. */
const message = computed(() => {
  if (lockedSeconds.value > 0) {
    return `Too many failed sign-ins. Try again in ${countdown.value}.`
  }
  if (props.phase.lockedUntil !== undefined) {
    return undefined
  }
  return (props.phase.error ?? props.phase.passkeyError)?.message
})

/** The request the shown failure came from; none for the lockout's countdown. */
const requestId = computed(() =>
  lockedSeconds.value > 0 || props.phase.lockedUntil !== undefined
    ? null
    : (props.phase.error ?? props.phase.passkeyError)?.requestId ?? null,
)

const canSubmit = computed(
  () =>
    !props.phase.busy &&
    !props.phase.passkeyBusy &&
    lockedSeconds.value === 0 &&
    isEmail(email.value) &&
    password.value.length > 0,
)

function submit() {
  if (!canSubmit.value) return
  emit('submit', email.value.trim(), password.value)
}

// A refused sign-in puts the reader back in the password, its text selected for retyping.
watch(
  () => props.phase.error,
  async (error) => {
    if (!error) {
      return
    }
    await nextTick()
    passwordField.value?.focus()
    passwordField.value?.select()
  },
)
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
          :autocomplete="passkeys ? 'username webauthn' : 'email'"
          inputmode="email"
          name="email"
          :readonly="phase.busy"
          focused
          @keydown.enter="submit"
        />
      </label>
      <label class="flex flex-col gap-1.5 text-chrome text-fg-muted">
        Password
        <TextInput
          ref="passwordField"
          v-model="password"
          size="lg"
          masked="password"
          autocomplete="current-password"
          name="password"
          :readonly="phase.busy"
          @keydown.enter="submit"
        />
      </label>
      <!-- The line keeps its height with nothing to say, so a message never moves the form. -->
      <div class="min-h-4">
        <InlineError v-if="message" :message="message" :request-id="requestId" />
      </div>
      <Button
        size="lg"
        variant="primary"
        class="w-full"
        :disabled="!canSubmit && !phase.busy"
        :loading="phase.busy"
        @click="submit"
      >
        Sign In
      </Button>
      <Button
        v-if="passkeys"
        size="lg"
        class="w-full"
        :disabled="phase.busy"
        :loading="phase.passkeyBusy"
        @click="emit('passkey')"
      >
        Sign In with Passkey
      </Button>
      <p class="select-none text-center text-[12px] leading-4 text-fg-subtle">
        Forgot your password? Ask your administrator to reset it.
      </p>
    </form>
  </AuthPage>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import Button from '../ui/Button.vue'
import AuthPage from './AuthPage.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { isEmail } from './email'

/**
 * Email and password on the auth page. Owns nothing; the host reports busy,
 * error, a lockout and why the user is here.
 *
 * While a sign-in runs the fields keep the focus, read-only. A wrong password
 * gives the focus back to the password field with its text selected, as
 * macOS does, so the next try is typed at once; the line under the fields
 * keeps its height either way, so nothing moves. A lockout counts down to its
 * end and keeps Sign In off until then.
 */
export type EmailLoginPhase = {
  busy?: boolean
  error?: string
  /** The address is locked until this time, in milliseconds since the epoch. */
  lockedUntil?: number
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
  return props.phase.lockedUntil === undefined ? props.phase.error : undefined
})

const canSubmit = computed(
  () =>
    !props.phase.busy &&
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
          autocomplete="email"
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
          secret
          autocomplete="current-password"
          name="password"
          :readonly="phase.busy"
          @keydown.enter="submit"
        />
      </label>
      <!-- The line keeps its height with nothing to say, so a message never moves the form. -->
      <div class="min-h-4">
        <InlineError v-if="message" :message="message" />
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
      <p class="select-none text-center text-[12px] leading-4 text-fg-subtle">
        Forgot your password? Ask your administrator to reset it.
      </p>
    </form>
  </AuthPage>
</template>

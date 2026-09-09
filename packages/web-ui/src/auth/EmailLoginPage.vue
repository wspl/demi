<script setup lang="ts">
import { computed } from 'vue'
import Button from '../ui/Button.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { isEmail } from './email'

/**
 * Email and password on the left, a wide empty intro on the right.
 * Owns nothing; the host reports busy, error, and why the user is here.
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
  <div class="flex h-full min-h-0 bg-surface-base text-fg">
    <section
      class="flex h-full w-full shrink-0 flex-col md:w-[28rem] lg:w-[32rem]"
    >
      <div class="select-none px-8 pt-8 lg:px-14">
        <span class="text-chrome font-medium text-fg-emphasis">Demi</span>
      </div>
      <div
        class="flex min-h-0 flex-1 flex-col justify-center px-8 py-12 lg:px-14"
      >
        <header class="select-none">
          <h1 class="text-[22px] font-medium text-fg-emphasis">Sign in</h1>
          <p class="mt-1 text-[13px] leading-5 text-fg-muted">
            <template v-if="phase.reason === 'expired'"
              >Your session ended. Sign in again.</template
            >
            <template v-else>Sign in with your email.</template>
          </p>
        </header>
        <form class="mt-8 flex flex-col gap-4" @submit.prevent="submit">
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
            Sign in
          </Button>
        </form>
      </div>
    </section>
    <aside class="relative hidden min-w-0 flex-1 bg-surface md:block">
      <div class="flex h-full flex-col justify-center px-16 lg:px-24">
        <h2
          class="max-w-lg select-none text-[32px] font-medium leading-10 text-fg-emphasis"
        >
          A place to think and build
        </h2>
        <p
          class="mt-4 max-w-md select-none text-[15px] leading-6 text-fg-muted"
        >
          Everything starts with a conversation.
        </p>
      </div>
    </aside>
  </div>
</template>

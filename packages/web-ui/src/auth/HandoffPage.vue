<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import AuthPage from './AuthPage.vue'
import Button from '../ui/Button.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import InlineError from '../ui/InlineError.vue'

/**
 * The page a phone's camera opens from a handoff code (`product.md` § User
 * system): it says it opens the page while the host spends the code, then
 * the host goes on to the page the code opens, signed in to its account.
 * A code that expired or was used says so, with Sign In, which the host
 * opens the sign-in page for; another failure says what failed.
 */
export type HandoffPagePhase =
  | { kind: 'opening' }
  | { kind: 'expired' }
  | ({ kind: 'failed' } & ShownError)

defineProps<{
  phase: HandoffPagePhase
}>()

const emit = defineEmits<{
  signIn: []
}>()
</script>

<template>
  <AuthPage
    v-if="phase.kind === 'opening'"
    title="Handing Off"
    message="Opening the page from your other device."
  >
    <div class="flex items-center gap-2 text-[13px] text-fg-muted" role="status">
      <IndeterminateSpinner />
      Opening…
    </div>
  </AuthPage>
  <AuthPage
    v-else
    title="Couldn’t Hand Off"
    :message="
      phase.kind === 'expired'
        ? 'This code expired or was already used. Show a new code, or sign in here.'
        : 'The code didn’t work. Show a new code, or sign in here.'
    "
  >
    <div class="flex flex-col items-start gap-4">
      <InlineError v-if="phase.kind === 'failed'" :message="phase.message" :request-id="phase.requestId" />
      <Button variant="primary" @click="emit('signIn')">Sign In</Button>
    </div>
  </AuthPage>
</template>

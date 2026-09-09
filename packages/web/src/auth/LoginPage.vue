<script setup lang="ts">
import { onUnmounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import EmailLoginPage, {
  type EmailLoginPhase,
} from '@demicodes/web-ui/auth/EmailLoginPage.vue'
import { useSession } from './session'

const session = useSession()
const router = useRouter()
const email = ref('')
const password = ref('')
const initial = session.current
const expired = useRoute().query.reason === 'expired'
const phase = ref<EmailLoginPhase>(
  initial.status === 'signedOut'
    ? {
        reason: expired ? 'expired' : initial.reason,
        error: initial.error,
      }
    : {},
)
let request: AbortController | null = null

async function submit(address: string, secret: string): Promise<void> {
  if (phase.value.busy) {
    return
  }
  const controller = new AbortController()
  request = controller
  phase.value = {
    busy: true,
    reason: phase.value.reason,
  }
  try {
    await session.signIn(address, secret, controller.signal)
    password.value = ''
    await router.replace('/chat')
  } catch (error) {
    if (controller.signal.aborted) {
      return
    }
    phase.value = {
      reason: phase.value.reason,
      error: error instanceof Error ? error.message : 'Sign-in failed.',
    }
  } finally {
    if (request === controller) {
      request = null
    }
  }
}

onUnmounted(() => request?.abort())
</script>

<template>
  <EmailLoginPage
    v-model:email="email"
    v-model:password="password"
    :phase="phase"
    @submit="submit"
  />
</template>

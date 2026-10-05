<script setup lang="ts">
import { onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import SetupForm, { type SetupPhase } from '@demicodes/web-ui/auth/SetupPage.vue'
import { ApiError } from '../api/client'
import { useSession } from './session'

const session = useSession()
const router = useRouter()
const name = ref('')
const email = ref('')
const password = ref('')
const confirmation = ref('')
const phase = ref<SetupPhase>({})
let request: AbortController | null = null

async function submit(nickname: string, address: string, secret: string): Promise<void> {
  if (phase.value.busy) {
    return
  }
  const controller = new AbortController()
  request = controller
  phase.value = { busy: true }
  try {
    await session.setUp(nickname, address, secret, controller.signal)
    password.value = ''
    confirmation.value = ''
    await router.replace('/chat')
  } catch (error) {
    if (controller.signal.aborted) {
      return
    }
    if (error instanceof ApiError && error.code === 'already_set_up') {
      password.value = ''
      confirmation.value = ''
      phase.value = { taken: true }
      return
    }
    phase.value = {
      error: error instanceof Error ? error.message : 'Setup failed.',
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
  <SetupForm
    v-model:name="name"
    v-model:email="email"
    v-model:password="password"
    v-model:confirmation="confirmation"
    :phase="phase"
    @submit="submit"
    @sign-in="router.replace('/login')"
  />
</template>

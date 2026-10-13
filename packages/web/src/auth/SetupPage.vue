<script setup lang="ts">
import { shownError } from '@demicodes/web-ui/infra/errors'
import { onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import SetupForm, { type SetupPhase } from '@demicodes/web-ui/auth/SetupPage.vue'
import { ApiError } from '../api/client'
import { addPasskey } from './passkeys'
import { setupStatus, useSession } from './session'
import { browserHasPasskeys } from './webauthn'

const session = useSession()
const router = useRouter()
const name = ref('')
const email = ref('')
const password = ref('')
const confirmation = ref('')
const phase = ref<SetupPhase>({})
let request: AbortController | null = null

/** Leaves setup for the app, the password typed forgotten. */
async function finish(): Promise<void> {
  password.value = ''
  confirmation.value = ''
  session.finishSetup()
  await router.replace('/chat')
}

/** Whether this server and this browser offer passkeys; a server that does not say offers none. */
async function passkeysOffered(signal: AbortSignal): Promise<boolean> {
  try {
    return browserHasPasskeys() && (await setupStatus(signal)).passkeyRpId != null
  } catch {
    signal.throwIfAborted()
    return false
  }
}

async function submit(nickname: string, address: string, secret: string): Promise<void> {
  if (phase.value.busy) {
    return
  }
  const controller = new AbortController()
  request = controller
  phase.value = { busy: true }
  try {
    // The new account adds its first passkey with the password it has just
    // entered, if it wants one.
    const offered = await passkeysOffered(controller.signal)
    await session.setUp(nickname, address, secret, controller.signal, offered)
    if (offered) {
      phase.value = { passkey: {} }
      return
    }
    await finish()
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
      error: shownError(error),
    }
  } finally {
    if (request === controller) {
      request = null
    }
  }
}

async function addFirstPasskey(): Promise<void> {
  if (phase.value.passkey?.busy) {
    return
  }
  phase.value = { passkey: { busy: true } }
  const outcome = await addPasskey(password.value)
  switch (outcome.kind) {
    case 'added':
      await finish()
      return
    case 'dismissed':
      phase.value = { passkey: {} }
      return
    case 'wrongPassword':
      phase.value = { passkey: { error: { message: outcome.message, requestId: null } } }
      return
    case 'failed':
      phase.value = { passkey: { error: outcome } }
      return
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
    @add-passkey="addFirstPasskey"
    @skip-passkey="finish"
  />
</template>

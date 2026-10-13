<script setup lang="ts">
import { formError, shownError } from '@demicodes/web-ui/infra/errors'
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import EmailLoginPage, {
  type EmailLoginPhase,
} from '@demicodes/web-ui/auth/EmailLoginPage.vue'
import { setupStatus, useSession } from './session'
import { returnAddress } from './return'
import { autofillOffersPasskeys, browserHasPasskeys } from './webauthn'
import { ApiError } from '../api/client'

const session = useSession()
const router = useRouter()
// A development build fills in the local test account so a local backend is one click away.
const devAccount = import.meta.env.DEV
  ? {
      email: import.meta.env.DEMI_DEV_EMAIL ?? '',
      password: import.meta.env.DEMI_DEV_PASSWORD ?? '',
    }
  : null
const email = ref(devAccount?.email ?? '')
const password = ref(devAccount?.password ?? '')
const initial = session.current
const route = useRoute()
const expired = route.query.reason === 'expired'
const phase = ref<EmailLoginPhase>(
  initial.status === 'signedOut'
    ? {
        reason: expired ? 'expired' : initial.reason,
      }
    : {},
)
/** Whether the server and the browser offer passkeys, once the server says so. */
const passkeys = ref(false)
let request: AbortController | null = null
/** The passkey request the email field's autofill answers, while one waits. */
let autofill: AbortController | null = null
/** The page is gone: nothing more starts. */
const lifetime = new AbortController()

async function signedIn(): Promise<void> {
  password.value = ''
  await router.replace(returnAddress(route.query))
}

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
    await signedIn()
  } catch (error) {
    if (controller.signal.aborted) {
      return
    }
    const wait = error instanceof ApiError && error.code === 'too_many_attempts' ? error.retryAfterSeconds : null
    phase.value = {
      reason: phase.value.reason,
      error: formError(error, {
        invalid_credentials: 'The email or password is incorrect.',
        account_disabled: 'This account is disabled. Ask your administrator.',
      }),
      ...(wait === null ? {} : { lockedUntil: Date.now() + wait * 1000 }),
    }
  } finally {
    if (request === controller) {
      request = null
    }
  }
}

/** Why a passkey sign-in failed, in the line under the fields. */
function passkeyFailed(error: unknown): void {
  phase.value = {
    reason: phase.value.reason,
    passkeyError: shownError(error),
  }
}

/**
 * Offers the account's passkeys in the email field's autofill, which waits
 * until the user picks one. A pick that fails says why and offers nothing
 * more by itself: Sign In with Passkey, the user's next try, offers them
 * again once it ends, so no failure repeats without the user.
 */
async function offerInAutofill(): Promise<void> {
  if (lifetime.signal.aborted || autofill || !(await autofillOffersPasskeys())) {
    return
  }
  const controller = new AbortController()
  autofill = controller
  try {
    if (await session.signInWithPasskey(true, controller.signal)) {
      await signedIn()
    }
  } catch (error) {
    if (!controller.signal.aborted) {
      passkeyFailed(error)
    }
  } finally {
    if (autofill === controller) {
      autofill = null
    }
  }
}

/** Sign In with Passkey: the browser's own prompt, in place of the autofill's request. */
async function signInWithPasskey(): Promise<void> {
  if (phase.value.passkeyBusy || phase.value.busy) {
    return
  }
  autofill?.abort()
  autofill = null
  const controller = new AbortController()
  request = controller
  phase.value = { reason: phase.value.reason, passkeyBusy: true }
  try {
    if (await session.signInWithPasskey(false, controller.signal)) {
      await signedIn()
      return
    }
    phase.value = { reason: phase.value.reason }
  } catch (error) {
    if (controller.signal.aborted) {
      return
    }
    passkeyFailed(error)
  } finally {
    if (request === controller) {
      request = null
    }
  }
  void offerInAutofill()
}

onMounted(async () => {
  let rpId: string | null
  try {
    rpId = (await setupStatus(lifetime.signal)).passkeyRpId ?? null
  } catch {
    // A server that does not answer offers no passkeys; the password's
    // sign-in reports what fails.
    return
  }
  passkeys.value = rpId !== null && browserHasPasskeys()
  if (passkeys.value) {
    // The request finds the field by its `webauthn` autocomplete token,
    // which the field has once it shows the passkeys.
    await nextTick()
    void offerInAutofill()
  }
})

onUnmounted(() => {
  lifetime.abort()
  request?.abort()
  autofill?.abort()
  autofill = null
})
</script>

<template>
  <EmailLoginPage
    v-model:email="email"
    v-model:password="password"
    :phase="phase"
    :passkeys="passkeys"
    @submit="submit"
    @passkey="signInWithPasskey"
  />
</template>

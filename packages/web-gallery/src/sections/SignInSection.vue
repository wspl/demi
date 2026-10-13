<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import EmailLoginPage, { type EmailLoginPhase } from '@demicodes/web-ui/auth/EmailLoginPage.vue'
import SetupPage, { type SetupPhase } from '@demicodes/web-ui/auth/SetupPage.vue'
import HandoffPage, { type HandoffPagePhase } from '@demicodes/web-ui/auth/HandoffPage.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import StartingScreen from '@demicodes/web-ui/ui/StartingScreen.vue'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { useGalleryView } from '../gallery-views'
import { productWould } from '../product-would'

const { view } = useGalleryView()

const anatomy: [string, string][] = [
  [
    'Layout',
    'A split page in the product language: the form on the base surface, a wide empty intro on the session surface. Below a tablet width the intro hides and the form fills the page. Sign-in and setup share it.'
  ],
  [
    'Sign in',
    'Email and password. The host reports busy, a wrong-credentials error, a lockout and whether the session ended. A wrong password gives the focus back to the password, its text selected, and the line under the fields keeps its height, so nothing moves. A lockout counts down to its end with Sign In off. No registration; a forgotten password goes to the administrator, as the line under Sign In says.'
  ],
  [
    'Passkeys',
    'Where the server and the browser offer passkeys, the email field lists them in its autofill and Sign In with Passkey asks the browser for one; neither needs the email first. A passkey that fails says why in the same line under the fields. A server at an IP address, or a browser without passkeys, shows neither.'
  ],
  [
    'Setup',
    'What the first visitor of a Demi without accounts sees instead of signing in: a name, an email, a password and its confirmation create the master account, which signs in. Where passkeys are offered, an optional Add a Passkey step follows, using the password just entered, with Skip. Once the account exists, the page is gone and every visitor signs in.'
  ],
  [
    'Phone handoff',
    'What a phone’s camera opens from a code another signed-in browser shows: it says it opens the page while the code is spent, then opens the page the code names, signed in: in the phone’s own session when it was signed in to the account. A code that expired or was used says so, with Sign In.'
  ],
  [
    'Starting',
    'What a page shows where the app will be until it knows who is signed in. While the page cannot reach Demi, as when it is loaded during a restart, it says it connects and waits; it shows the sign-in page only when Demi answers that nobody is signed in, and any other failure Demi answers with Retry.'
  ],
]

const states: {
  value: string;
  label: TitleText;
  phase: EmailLoginPhase
}[] = [
  { value: 'form', label: 'Form', phase: {} },
  { value: 'busy', label: 'Signing In', phase: { busy: true } },
  {
    value: 'error',
    label: 'Wrong Password',
    phase: { error: 'Wrong email or password' }
  },
  {
    value: 'locked',
    label: 'Locked',
    // The lock's end is set when the state is shown, a minute on.
    phase: {
      error: 'Too many failed sign-ins. Try again in a minute.'
    }
  },
  { value: 'expired', label: 'Session Ended', phase: { reason: 'expired' } },
  { value: 'passkey', label: 'Passkey', phase: { passkeyBusy: true } },
  {
    value: 'passkey-error',
    label: 'Unknown Passkey',
    phase: { passkeyError: 'This passkey is no longer on any account. Sign in with your password.' },
  },
]

/** Whether the server and the browser offer passkeys. */
const passkeysOffered = ref(true)
const offerOptions: { value: 'offered' | 'none'; label: TitleText }[] = [
  { value: 'offered', label: 'Passkeys' },
  { value: 'none', label: 'No Passkeys' },
]

/** Sign In with Passkey: the browser's prompt the product would open, then the chat. */
async function signInWithPasskey() {
  state.value = 'passkey'
  await new Promise((resolve) => setTimeout(resolve, 900))
  state.value = 'form'
  productWould('Ask the Browser for a Passkey, Sign In and Open the Chat')
}

const state = ref('form')
const email = ref('zan@example.com')
const password = ref('password')
/** When the lock shown ends, a minute after the state was chosen, as the backend's Retry-After says. */
const lockedUntil = ref(0)
watch(state, (value) => {
  if (value === 'locked') {
    lockedUntil.value = Date.now() + 60_000
  }
})
const phase = computed(() => {
  const chosen = states.find((entry) => entry.value === state.value)!.phase
  return state.value === 'locked' ? { ...chosen, lockedUntil: lockedUntil.value } : chosen
})
let failures = 0

// The prototype accepts any password but `wrong`; five failures lock it.
function signIn(_: string, secret: string) {
  if (secret !== 'wrong') {
    failures = 0
    state.value = 'form'
    productWould('Sign In and Open the Chat')
    return
  }
  failures += 1
  state.value = failures >= 5 ? 'locked' : 'error'
}

/** A session check the gallery simulates: it fails again after a moment, as a backend that still fails it would. */
function checkAgain(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 1500))
}

const setupStates: {
  value: string
  label: TitleText
  phase: SetupPhase
  /** The fields the state shows, when it is about what was typed. */
  typed?: { password: string; confirmation: string }
}[] = [
  { value: 'form', label: 'Form', phase: {} },
  { value: 'busy', label: 'Creating', phase: { busy: true } },
  {
    value: 'short',
    label: 'Too Short',
    phase: {},
    typed: { password: 'short', confirmation: '' },
  },
  {
    value: 'differ',
    label: 'Passwords Differ',
    phase: {},
    typed: { password: 'correct horse', confirmation: 'correct hose' },
  },
  {
    value: 'error',
    label: 'Error',
    phase: { error: 'Couldn’t reach Demi. Check the connection and try again.' },
  },
  { value: 'taken', label: 'Set Up Already', phase: { taken: true } },
  { value: 'passkey', label: 'Add a Passkey', phase: { passkey: {} } },
  { value: 'passkey-busy', label: 'Adding Passkey', phase: { passkey: { busy: true } } },
  {
    value: 'passkey-error',
    label: 'Passkey Failed',
    phase: { passkey: { error: 'The passkey could not be verified' } },
  },
]

/** Add Passkey on setup's step: the browser's prompt the product would open, then the chat. */
async function addFirstPasskey() {
  setupState.value = 'passkey-busy'
  await new Promise((resolve) => setTimeout(resolve, 900))
  setupState.value = 'passkey'
  productWould('Ask the Browser to Save a Passkey and Open the Chat')
}

const setupState = ref('form')
const setupName = ref('Zan')
const setupEmail = ref('zan@example.com')
const setupPassword = ref('correct horse')
const setupConfirmation = ref('correct horse')
const setupPhase = computed(
  () => setupStates.find((entry) => entry.value === setupState.value)!.phase,
)

function showSetupState(value: string) {
  setupState.value = value
  const typed = setupStates.find((entry) => entry.value === value)!.typed
  setupPassword.value = typed?.password ?? 'correct horse'
  setupConfirmation.value = typed?.confirmation ?? 'correct horse'
}

const handoffStates: { value: string; label: TitleText; phase: HandoffPagePhase }[] = [
  { value: 'opening', label: 'Opening', phase: { kind: 'opening' } },
  { value: 'expired', label: 'Expired', phase: { kind: 'expired' } },
  { value: 'failed', label: 'Failed', phase: { kind: 'failed', message: 'Couldn’t reach Demi. Check the connection and try again.' } },
]
const handoffState = ref('expired')
const handoffPhase = computed(() => handoffStates.find((entry) => entry.value === handoffState.value)!.phase)

function createAccount(name: string, address: string) {
  productWould(`Create the Master Account ${name} (${address})`)
  setupState.value = 'passkey'
}
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection
      title="Sign In & Setup"
      note="The product’s entry pages: sign-in and setup, a form on the left and a large empty intro on the right, and what shows before the app starts."
    >
      <dl
        class="grid max-w-3xl grid-cols-[7rem_minmax(0,1fr)] gap-x-4 gap-y-2 text-[13px] leading-5"
      >
        <template v-for="[term, detail] in anatomy" :key="term">
          <dt class="select-none text-fg-subtle">{{ term }}</dt>
          <dd class="text-fg-muted">{{ detail }}</dd>
        </template>
      </dl>
    </GallerySection>

    <GallerySection
      v-if="view === 'signin'"
      title="Sign In"
      note="Switch the host-reported state, and whether passkeys are offered. The prototype accepts any password except wrong; five failures lock the form. Sign In with Passkey shows the passkey sign-in running, then what the product would do."
    >
      <div class="mb-3 flex flex-wrap gap-3">
        <Segmented v-model="state" :options="states" />
        <Segmented
          :model-value="passkeysOffered ? 'offered' : 'none'"
          :options="offerOptions"
          @update:model-value="passkeysOffered = $event === 'offered'"
        />
      </div>
      <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
        <EmailLoginPage
          v-model:email="email"
          v-model:password="password"
          :phase="phase"
          :passkeys="passkeysOffered"
          @submit="signIn"
          @passkey="signInWithPasskey"
        />
      </div>
    </GallerySection>

    <GallerySection
      v-else-if="view === 'setup'"
      title="Setup"
      note="Switch the state. Too short and differing passwords come from what is typed; the others the host reports. Set Up Already is what a visitor sees when another created the master account first. Create Account goes on to the optional Add a Passkey step."
    >
      <div class="mb-3">
        <Segmented
          :model-value="setupState"
          :options="setupStates"
          @update:model-value="showSetupState"
        />
      </div>
      <div class="h-[40rem] overflow-hidden rounded-xl border border-line">
        <SetupPage
          v-model:name="setupName"
          v-model:email="setupEmail"
          v-model:password="setupPassword"
          v-model:confirmation="setupConfirmation"
          :phase="setupPhase"
          @submit="createAccount"
          @sign-in="view = 'signin'"
          @add-passkey="addFirstPasskey"
          @skip-passkey="productWould('Open the Chat')"
        />
      </div>
    </GallerySection>

    <GallerySection
      v-else-if="view === 'handoff'"
      title="Phone Handoff"
      note="Switch the state. Opening shows while the code is spent; the product then opens the page the code names. Sign In opens the sign-in page."
    >
      <div class="mb-3">
        <Segmented v-model="handoffState" :options="handoffStates" />
      </div>
      <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
        <HandoffPage :phase="handoffPhase" @sign-in="productWould('Open the Sign-In Page')" />
      </div>
    </GallerySection>

    <GallerySection
      v-else
      title="Starting"
      note="The product’s StartingScreen, pinned in each state. Loading and Connecting go on by themselves once Demi answers; Failed offers Retry, which here fails again after a moment."
    >
      <div class="grid gap-6 xl:grid-cols-2">
        <GallerySpecimen wide variant="Loading · checking who is signed in">
          <div class="h-[20rem] overflow-hidden rounded-xl border border-line bg-surface-base">
            <StartingScreen class="h-full" :connecting="false" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Connecting · Demi cannot be reached yet">
          <div class="h-[20rem] overflow-hidden rounded-xl border border-line bg-surface-base">
            <StartingScreen class="h-full" connecting />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Failed · Demi answered the check with its own error">
          <div class="h-[20rem] overflow-hidden rounded-xl border border-line bg-surface-base">
            <!-- Retry checks again and, as the gallery has no backend, fails the same way after a moment. -->
            <StartingScreen
              class="h-full"
              :connecting="false"
              failure="The session store could not be read: database is locked"
              :on-retry="checkAgain"
            />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
  </div>
</template>

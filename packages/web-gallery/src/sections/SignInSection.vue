<script setup lang="ts">
import { computed, ref } from 'vue'
import EmailLoginPage, { type EmailLoginPhase } from '@demicodes/web-ui/auth/EmailLoginPage.vue'
import SetupPage, { type SetupPhase } from '@demicodes/web-ui/auth/SetupPage.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'
import GallerySection from '../components/GallerySection.vue'
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
    'Email and password. The host reports busy, a wrong-credentials or lockout error, and whether the session ended. No registration or password recovery.'
  ],
  [
    'Setup',
    'What the first visitor of a Demi without accounts sees instead of signing in: email, password and its confirmation create the master account, which signs in. Once it exists, the page is gone and every visitor signs in.'
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
    phase: {
      error: 'Too many failed logins; try again in a minute'
    }
  },
  { value: 'expired', label: 'Session Ended', phase: { reason: 'expired' } },
]

const state = ref('form')
const email = ref('zan@example.com')
const password = ref('password')
const phase = computed(() => states.find((entry) => entry.value === state.value)!.phase)
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
]

const setupState = ref('form')
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

function createAccount(address: string) {
  productWould(`Create the Master Account ${address} and Open the Chat`)
}
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection
      title="Sign In & Setup"
      note="The product’s entry pages. A form on the left; a large empty intro on the right."
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
      note="Switch the host-reported state. The prototype accepts any password except wrong; five failures lock the form."
    >
      <div class="mb-3"><Segmented v-model="state" :options="states" /></div>
      <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
        <EmailLoginPage
          v-model:email="email"
          v-model:password="password"
          :phase="phase"
          @submit="signIn"
        />
      </div>
    </GallerySection>

    <GallerySection
      v-else
      title="Setup"
      note="Switch the state. Too short and differing passwords come from what is typed; the others the host reports. Set Up Already is what a visitor sees when another created the master account first."
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
          v-model:email="setupEmail"
          v-model:password="setupPassword"
          v-model:confirmation="setupConfirmation"
          :phase="setupPhase"
          @submit="createAccount"
          @sign-in="view = 'signin'"
        />
      </div>
    </GallerySection>
  </div>
</template>

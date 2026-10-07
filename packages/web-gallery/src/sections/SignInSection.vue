<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import EmailLoginPage, { type EmailLoginPhase } from '@demicodes/web-ui/auth/EmailLoginPage.vue'
import SetupPage, { type SetupPhase } from '@demicodes/web-ui/auth/SetupPage.vue'
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
    'Setup',
    'What the first visitor of a Demi without accounts sees instead of signing in: a name, an email, a password and its confirmation create the master account, which signs in. Once it exists, the page is gone and every visitor signs in.'
  ],
  [
    'Starting',
    'What a page shows where the app will be until it knows who is signed in. While the page cannot reach Demi, as when it is loaded during a restart, it says it connects and waits; it shows the sign-in page only when Demi answers that nobody is signed in.'
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
]

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

function createAccount(name: string, address: string) {
  productWould(`Create the Master Account ${name} (${address}) and Open the Chat`)
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
      v-else-if="view === 'setup'"
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
          v-model:name="setupName"
          v-model:email="setupEmail"
          v-model:password="setupPassword"
          v-model:confirmation="setupConfirmation"
          :phase="setupPhase"
          @submit="createAccount"
          @sign-in="view = 'signin'"
        />
      </div>
    </GallerySection>

    <GallerySection
      v-else
      title="Starting"
      note="The product’s StartingScreen, pinned in each state. Nothing to click: the page goes on by itself once Demi answers."
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
      </div>
    </GallerySection>
  </div>
</template>

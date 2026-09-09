<script setup lang="ts">
import { computed, ref } from 'vue'
import EmailLoginPage, { type EmailLoginPhase } from '@demicodes/web-ui/auth/EmailLoginPage.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import GallerySection from '../components/GallerySection.vue'

const anatomy: [string, string][] = [
  [
    'Layout',
    'A split page in the product language: the form on the base surface, a wide empty intro on the session surface. Below a tablet width the intro hides and the form fills the page.'
  ],
  [
    'Form',
    'Email and password. The host reports busy, a wrong-credentials or lockout error, and whether the session ended. No registration or password recovery.'
  ],
]

const states: {
  value: string;
  label: string;
  phase: EmailLoginPhase
}[] = [
  { value: 'form', label: 'Form', phase: {} },
  { value: 'busy', label: 'Signing in', phase: { busy: true } },
  {
    value: 'error',
    label: 'Wrong password',
    phase: { error: 'Wrong email or password' }
  },
  {
    value: 'locked',
    label: 'Locked',
    phase: {
      error: 'Too many failed logins; try again in a minute'
    }
  },
  { value: 'expired', label: 'Session ended', phase: { reason: 'expired' } },
]

const state = ref('form')
const email = ref('zan@example.com')
const password = ref('password')
const phase = computed(() => states.find((entry) => entry.value === state.value)!.phase)
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection
      title="Sign in"
      note="The product entry page. Email and password on the left; a large empty intro on the right."
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
      title="Page"
      note="Switch the host-reported state. The prototype accepts any password except wrong; five failures lock the form."
    >
      <div class="mb-3"><Segmented v-model="state" :options="states" /></div>
      <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
        <EmailLoginPage
          v-model:email="email"
          v-model:password="password"
          :phase="phase"
        />
      </div>
    </GallerySection>
  </div>
</template>

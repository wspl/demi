<script setup lang="ts">
import { t } from './locales/text'
import { ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { PasskeyOfferOutcome, PasskeyOfferStep } from '../settings/types'
import Wizard from '../ui/Wizard.vue'
import PasskeyOfferSteps from './PasskeyOfferSteps.vue'

/**
 * The offer to add a passkey as a wizard of its own (`product.md` § User
 * system), which a host opens once on a browser after a sign-in that has
 * just proved the user: a handoff code's. Its steps are `PasskeyOfferSteps`,
 * which Choose Your Password goes on into in its own dialog.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  add: (password: string | null) => Promise<PasskeyOfferOutcome>
  /** The account's email, which the password manager files the password under. */
  email: string
}>()

const emit = defineEmits<{
  answered: []
  close: []
}>()

const step = ref<PasskeyOfferStep>('ask')
</script>

<template>
  <Wizard
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="t('offer.label')"
    :step="step"
    skippable
    @close="emit('close')"
  >
    <PasskeyOfferSteps
      v-model:step="step"
      :is-open="isOpen"
      :add="add"
      :email="email"
      @answered="emit('answered')"
      @close="emit('close')"
    />
  </Wizard>
</template>

<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import VerifyForm from './VerifyForm.vue'
import { useVerification, type VerificationHost } from './verification'
import type { SentenceText } from '../ui/ui-text'

/**
 * Confirm It’s You: the step an action that needs the user's own
 * confirmation asks for on its own, as Hand Off to Your Phone does inside
 * its dialog, with the same fields and buttons. `verified` is emitted once
 * the host confirmed, for the action to go on.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  host: VerificationHost
  /** The account's email, which the password manager files the password under. */
  email: string
  /** What the confirmation is for, said under the title. */
  message: SentenceText
}>()

const emit = defineEmits<{
  close: []
  verified: []
}>()

const verification = useVerification(
  () => props.host,
  () => emit('verified'),
)
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      verification.reset()
    }
  },
)
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="t('verify.title')" stack @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('verify.title') }}</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">{{ message }}</p>
      </header>
      <VerifyForm :verification="verification" :email="email" />
    </div>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button
        v-if="host.verifyPasskey"
        :operation="verification.withPasskey"
        @click="verification.usePasskey()"
      >
        {{ t('handoff.usePasskey') }}
      </Button>
      <Button
        variant="primary"
        :operation="verification.withPassword"
        @click="verification.submitPassword()"
      >
        {{ uiText('common.continue') }}
      </Button>
    </template>
  </Dialog>
</template>

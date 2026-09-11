<script setup lang="ts">
import type { CloudState } from './types'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import InlineError from '../ui/InlineError.vue'
import { resetPhaseLabels } from './reset-phases'

/**
 * The Cloud reset, confirmed and then followed step by step: what it stops,
 * what it keeps, the phase it is in, and how it ended. A failed reset stays in
 * the dialog with Retry reset, because the dialog is where the reset was asked for.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The reset's current step once submitted; null before it starts. */
  phase: CloudState['phase']
  /** The reset was requested from this dialog. */
  submitted: boolean
  /** Why the reset request itself failed, if it did. */
  error: string | null
  /** A reset is in flight: no second request. */
  busy: boolean
}>()
const emit = defineEmits<{
  close: []
  reset: []
}>()
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    label="Reset Cloud environment"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 p-5">
      <h3 class="pr-8 text-[15px] font-medium text-fg-emphasis">
        Reset Cloud environment
      </h3>
      <p class="text-[13px] leading-5 text-fg-muted">
        This stops all your Cloud tasks and replaces installed system packages and
        system settings. Files in your home directory, including every Cloud
        project, remain.
      </p>
      <p
        v-if="submitted && phase"
        role="status"
        aria-live="polite"
        class="text-[13px] text-fg-body"
      >
        {{ resetPhaseLabels[phase] }}
      </p>
      <InlineError v-if="submitted && error" :message="error" />
      <div class="flex justify-end gap-2">
        <Button @click="emit('close')">{{ submitted ? 'Close' : 'Cancel' }}</Button>
        <Button
          v-if="!submitted || error || phase === 'failed'"
          variant="danger"
          :disabled="busy"
          @click="emit('reset')"
          >{{ submitted ? 'Retry reset' : 'Reset environment' }}</Button
        >
      </div>
    </div>
  </Dialog>
</template>

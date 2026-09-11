<script setup lang="ts">
import { computed } from 'vue'
import { CircleCheck } from '@lucide/vue'
import type { CloudState } from './types'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import InlineError from '../ui/InlineError.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { resetPhaseLabels } from './reset-phases'

/**
 * The Cloud reset, confirmed and then followed step by step: what it stops,
 * what it keeps, and one status line that is a spinner while a step runs, a
 * check when the Cloud is back, or the failure with Retry reset. A failed
 * reset stays in the dialog, because the dialog is where it was asked for.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The reset's current step once submitted; null before it starts. */
  phase: CloudState['phase']
  /** The reset was requested from this dialog. */
  submitted: boolean
  /** Why the reset failed, if it did. */
  error: string | null
  /** A reset is in flight: no second request. */
  busy: boolean
}>()
const emit = defineEmits<{
  close: []
  reset: []
}>()

const failed = computed(() => props.submitted && (props.error !== null || props.phase === 'failed'))
const running = computed(
  () => props.submitted && !failed.value && props.phase !== null && props.phase !== 'ready',
)
const ready = computed(() => props.submitted && !failed.value && props.phase === 'ready')
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
        v-if="running && phase"
        role="status"
        aria-live="polite"
        class="flex items-center gap-2 text-chrome text-fg-muted"
      >
        <IndeterminateSpinner :size="ICON_PX.in24" />
        {{ resetPhaseLabels[phase] }}
      </p>
      <p
        v-else-if="ready"
        role="status"
        aria-live="polite"
        class="flex items-center gap-2 text-chrome text-on-success"
      >
        <CircleCheck :size="ICON_PX.in24" aria-hidden="true" />
        {{ resetPhaseLabels.ready }}
      </p>
      <InlineError v-else-if="failed" :message="error ?? resetPhaseLabels.failed" />
      <div class="flex justify-end gap-2">
        <Button @click="emit('close')">{{ submitted ? 'Close' : 'Cancel' }}</Button>
        <Button
          v-if="!submitted || failed"
          variant="danger"
          :disabled="busy"
          @click="emit('reset')"
          >{{ submitted ? 'Retry reset' : 'Reset environment' }}</Button
        >
      </div>
    </div>
  </Dialog>
</template>

<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import Dialog from './Dialog.vue'
import type { HeadlineText } from './ui-text'

/**
 * A choice the user must make, step by step (`web-application.md` §
 * Responding to the user): on a computer one dialog at the centre of the
 * page over the dimmed page, on a phone the whole screen, as iOS's own setup
 * is. The steps replace each other in place, each a `WizardStep`, which the
 * host names by `step` so a change of step plays the change; the frame stays
 * where it is. A step that cannot be skipped has no close button and Escape
 * and the scrim leave it; a skippable one closes from its corner, by Escape
 * and by the scrim, which `close` tells the host to take as the step's own
 * way out. Choose Your Password and the passkey offer are its steps.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The wizard's accessible name. */
  label: HeadlineText
  /** The step shown, by the host's own name for it. */
  step: string
  /** Whether the step can be skipped, which shows the close button and lets Escape and the scrim close it. */
  skippable?: boolean
}>()

const emit = defineEmits<{
  close: []
}>()
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="label"
    :hide-close="!skippable"
    fill-narrow
    @close="skippable && emit('close')"
  >
    <Transition name="wizard-step" mode="out-in">
      <div :key="step" class="flex min-h-full flex-1 flex-col">
        <slot />
      </div>
    </Transition>
  </Dialog>
</template>

<style scoped>
.wizard-step-enter-active,
.wizard-step-leave-active {
  transition: opacity 120ms ease-out;
}

.wizard-step-enter-from,
.wizard-step-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .wizard-step-enter-active,
  .wizard-step-leave-active {
    transition: none;
  }
}
</style>

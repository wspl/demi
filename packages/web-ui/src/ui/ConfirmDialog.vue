<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import Button from './Button.vue'
import Dialog from './Dialog.vue'
import type { HeadlineText, TitleText } from './ui-text'

/**
 * The question before something the user set up goes, as macOS asks before
 * it deletes: the title asks it ("Remove “OpenAI API”?"), the body says what
 * happens, and `goes` names what goes with it, one line each. Cancel is the
 * safe answer, the default button, and closes; the action is the danger
 * button, which takes a click (Dialog). Nothing the user
 * set up disappears on one click (`product.md`). While the host carries the
 * action out, `busy` keeps the dialog open with its action loading.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  title: HeadlineText
  /** The danger button's label: Remove, Delete, Revoke. */
  action: TitleText
  /** What goes with it, named one per line; content, not UI text. */
  goes?: readonly string[]
  busy?: boolean
}>()

const emit = defineEmits<{
  close: []
  confirm: []
}>()
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="title"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <h3 class="pr-8 text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
      <div class="flex flex-col gap-3 text-[13px] leading-5 text-fg-muted">
        <slot />
      </div>
      <ul
        v-if="goes?.length"
        class="flex flex-col gap-1 pl-4 text-[13px] leading-5 text-fg-emphasis"
      >
        <li v-for="(entry, index) in goes" :key="index" class="list-disc break-words">
          {{ entry }}
        </li>
      </ul>
    </div>
    <template #footer>
      <!-- Cancel is the default: Return and the opening focus never destroy (Dialog). -->
      <Button :disabled="busy" variant="primary" @click="emit('close')">Cancel</Button>
      <Button variant="danger" :loading="busy" @click="emit('confirm')">{{ action }}</Button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import Button from './Button.vue'
import Dialog from './Dialog.vue'
import type { HeadlineText, SentenceText } from './ui-text'

/**
 * A sheet of what something saw, as labels and values, as macOS's Network →
 * Details… opens one: diagnostics that would crowd the page they explain.
 * A value of several lines is a list; Done closes it.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  title: HeadlineText
  entries: readonly { label: SentenceText; value: string | readonly string[] }[]
}>()
const emit = defineEmits<{
  close: []
}>()
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="title" @close="emit('close')">
    <div class="flex flex-col gap-4 p-5">
      <header class="select-none pr-8">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
      </header>
      <dl class="grid select-text grid-cols-[max-content_minmax(0,1fr)] gap-x-6 gap-y-2 text-[13px] leading-5">
        <template v-for="entry in entries" :key="entry.label">
          <dt class="text-fg-subtle">{{ entry.label }}</dt>
          <dd class="break-words text-fg">
            <template v-if="typeof entry.value === 'string'">{{ entry.value }}</template>
            <span v-for="(line, index) in entry.value" v-else :key="index" class="block">{{ line }}</span>
          </dd>
        </template>
      </dl>
      <div class="flex justify-end">
        <Button variant="primary" @click="emit('close')">Done</Button>
      </div>
    </div>
  </Dialog>
</template>

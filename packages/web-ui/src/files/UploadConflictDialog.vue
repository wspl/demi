<script setup lang="ts">
import { computed } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'

/**
 * The question before an upload writes over files: the picked names a
 * directory already has. Replace uploads every picked file, writing over
 * those; Skip uploads only the others; closing uploads nothing.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The picked names already in the directory. */
  names: readonly string[]
  /** The directory, as it reads from the workspace. */
  directory: string
  /** How many picked files the directory does not have yet. */
  others: number
}>()

const emit = defineEmits<{
  replace: []
  skip: []
  cancel: []
}>()

const title = computed(() => props.names.length === 1 ? `Replace ${props.names[0]}?` : `Replace ${props.names.length} files?`)

/** The names in a sentence: all of them up to three, then the first two and how many more. */
const listed = computed(() => {
  const names = props.names
  if (names.length <= 3)
    return new Intl.ListFormat('en', { type: 'conjunction' }).format(names)
  return `${names.slice(0, 2).join(', ')} and ${names.length - 2} more`
})

const body = computed(() => {
  const one = props.names.length === 1
  const them = one ? 'it' : 'them'
  const rest = props.others === 0 ? 'nothing' : `the other ${props.others === 1 ? 'file' : `${props.others} files`}`
  return `${listed.value} ${one ? 'is' : 'are'} already in ${props.directory}. Replace writes over ${them}; Skip leaves ${them} and uploads ${rest}.`
})
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="title"
    @close="emit('cancel')"
  >
    <div class="flex flex-col gap-4 p-5">
      <h3 class="pr-8 text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
      <p class="text-[13px] leading-5 text-fg-muted">{{ body }}</p>
      <div class="flex justify-end gap-2">
        <Button @click="emit('skip')">Skip</Button>
        <Button variant="danger" @click="emit('replace')">Replace</Button>
      </div>
    </div>
  </Dialog>
</template>

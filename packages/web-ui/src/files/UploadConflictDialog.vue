<script setup lang="ts">
import { computed } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import type { UploadClash } from './file-uploads'

/**
 * The question before an upload writes over what a directory has: the
 * dropped or picked names it already has. Replace puts each in place of what
 * is there; Merge, offered once a folder meets a folder, adds a folder's
 * files to the folder there; Skip uploads only the others; closing uploads
 * nothing.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The names the directory has, with what each is on both sides. */
  clashes: readonly UploadClash[]
  /** The directory, as it reads from the workspace. */
  directory: string
  /** How many of what else is going the directory does not have yet. */
  others: { files: number; folders: number }
}>()

const emit = defineEmits<{
  replace: []
  merge: []
  skip: []
  cancel: []
}>()

const merges = computed(() => props.clashes.some((clash) => clash.isDirectory && clash.takenByDirectory))

/** `file`, `folder`, or `item` for a mix, in the plural past one. */
function noun(files: number, folders: number): string {
  const word = folders === 0 ? 'file' : files === 0 ? 'folder' : 'item'
  return files + folders === 1 ? word : `${word}s`
}

const title = computed(() => {
  const clashes = props.clashes
  if (clashes.length === 1)
    return `Replace ${clashes[0]!.name}?`
  const folders = clashes.filter((clash) => clash.isDirectory).length
  return `Replace ${clashes.length} ${noun(clashes.length - folders, folders)}?`
})

/** The names in a sentence: all of them up to three, then the first two and how many more. */
const listed = computed(() => {
  const names = props.clashes.map((clash) => clash.name)
  if (names.length <= 3)
    return new Intl.ListFormat('en', { type: 'conjunction' }).format(names)
  return `${names.slice(0, 2).join(', ')} and ${names.length - 2} more`
})

const body = computed(() => {
  const one = props.clashes.length === 1
  const them = one ? 'it' : 'them'
  const { files, folders } = props.others
  const count = files + folders
  const rest = count === 0 ? 'nothing' : `the other ${count === 1 ? '' : `${count} `}${noun(files, folders)}`
  const answers = merges.value
    ? `Merge adds yours to ${them}, writing over files with the same names; Replace deletes ${them} with everything in ${them} first`
    : `Replace writes over ${them}`
  return `${listed.value} ${one ? 'is' : 'are'} already in ${props.directory}. ${answers}; Skip leaves ${them} and uploads ${rest}.`
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
        <Button v-if="merges" @click="emit('merge')">Merge</Button>
        <Button variant="danger" @click="emit('replace')">Replace</Button>
      </div>
    </div>
  </Dialog>
</template>

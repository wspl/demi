<script setup lang="ts">
import { computed } from 'vue'
import { pageLanguage } from '../i18n/text'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import { formatCount } from '../ui/numbers'
import type { UploadClash } from './file-uploads'
import { t } from './locales/text'

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

/** What a count of names or items is: only files, only folders, or a mix. */
type Kind = 'files' | 'folders' | 'items'

function kindOf(files: number, folders: number): Kind {
  return folders === 0 ? 'files' : files === 0 ? 'folders' : 'items'
}

/** A name as the dialog quotes it. */
function quoted(name: string): string {
  return t('conflict.item', { name })
}

const title = computed(() => {
  const clashes = props.clashes
  if (clashes.length === 1)
    return t('conflict.titleOne', { name: quoted(clashes[0]!.name) })
  const folders = clashes.filter((clash) => clash.isDirectory).length
  const count = clashes.length
  const named = { count: formatCount(count) }
  switch (kindOf(count - folders, folders)) {
    case 'files':
      return t('conflict.titleFiles', named, count)
    case 'folders':
      return t('conflict.titleFolders', named, count)
    case 'items':
      return t('conflict.titleItems', named, count)
  }
})

/** The names in a sentence: all of them up to three, then the first two and how many more. */
const listed = computed(() => {
  const names = props.clashes.map((clash) => quoted(clash.name))
  if (names.length <= 3)
    return new Intl.ListFormat(pageLanguage(), { type: 'conjunction' }).format(names)
  return t('conflict.namesMore', { first: names[0]!, second: names[1]!, count: formatCount(names.length - 2) })
})

/** What Skip does: uploads nothing, or only what else is going. */
const skipSentence = computed(() => {
  const { files, folders } = props.others
  const count = files + folders
  const named = { count: formatCount(count) }
  if (count === 0)
    return t('conflict.skipNothing')
  switch (kindOf(files, folders)) {
    case 'files':
      return t('conflict.skipFiles', named, count)
    case 'folders':
      return t('conflict.skipFolders', named, count)
    case 'items':
      return t('conflict.skipItems', named, count)
  }
})

const body = computed(() => {
  const one = props.clashes.length === 1
  const named = { names: listed.value, directory: props.directory }
  const already = one ? t('conflict.alreadyOne', named) : t('conflict.alreadyMany', named)
  const answer = merges.value
    ? one ? t('conflict.mergeOne') : t('conflict.mergeMany')
    : one ? t('conflict.replaceOne') : t('conflict.replaceMany')
  return t('conflict.body', { already, answer, skip: skipSentence.value })
})
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="title"
    @close="emit('cancel')"
  >
    <div class="flex flex-col gap-4 px-5 pt-5">
      <h3 class="pr-8 text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
      <p class="text-explanation leading-5 text-fg-muted">{{ body }}</p>
    </div>
    <template #footer>
      <Button @click="emit('skip')">{{ t('conflict.skip') }}</Button>
      <Button v-if="merges" @click="emit('merge')">{{ t('conflict.merge') }}</Button>
      <Button variant="danger" @click="emit('replace')">{{ t('conflict.replace') }}</Button>
    </template>
  </Dialog>
</template>

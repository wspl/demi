<script setup lang="ts">
import { reactive } from 'vue'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import SelectionField from '@demicodes/web-ui/agent/comments/SelectionField.vue'
import { t } from '@demicodes/web-ui/agent/locales/text'
import { productWould } from '../product-would'
import { galleryMessageFiles } from '../fixtures/written-files'

/**
 * The comment field pinned in the states it has, each live: its own words,
 * every action saying what the product would do with the selection, and
 * Escape closing it, whereupon it comes back so it can be tried again. A
 * press of a button or Return with no words shakes the field once. Hold ⌘
 * (Control elsewhere) in any of them to turn the blue button into Send. A
 * file pasted or dropped on a field lands where it was put and uploads at a
 * pace slow enough to watch.
 */
const page = galleryMessageFiles()
const states = reactive([
  { label: 'No words', message: { text: '', fileIds: [] as string[] }, way: 'comment' as const },
  { label: 'With words: Return comments, ⌘Return sends', message: { text: 'Is 4 MB right for a slow link?', fileIds: [] as string[] }, way: 'comment' as const },
  { label: 'Ask chosen from the menu: the blue button asks', message: { text: 'Why 4 MB?', fileIds: [] as string[] }, way: 'ask' as const },
  { label: '⌘ held: the blue button sends', message: { text: 'Is 4 MB right for a slow link?', fileIds: [] as string[] }, way: 'comment' as const, held: true },
  { label: 'Words past the line: the buttons go under them', message: { text: 'Would 4 MB chunks still fit in the timeout on the slowest 3G links we see?', fileIds: [] as string[] }, way: 'comment' as const },
  { label: 'With a file: its capsule where it was added', message: withFile('Are these limits right for a slow link? '), way: 'comment' as const },
  { label: 'The reply is still being written', message: { text: 'Is 4 MB right for a slow link?', fileIds: [] as string[] }, way: 'comment' as const, unavailable: true },
])
const shown = reactive(states.map(() => true))

/** `text` with a text file's capsule after it, the file uploading as one added does. */
function withFile(text: string): { text: string; fileIds: string[] } {
  const [capsule] = page.files.add([new File(['Uploads go in 4 MB chunks with a 30-second timeout each.\n'], 'upload-limits.md', { type: 'text/markdown' })])
  return { text: `${text}${ATTACHMENT_MARK}`, fileIds: capsule ? [capsule.id] : [] }
}

function close(index: number): void {
  shown[index] = false
  productWould('Close the Field and Keep the Selection')
  setTimeout(() => (shown[index] = true), 600)
}
</script>

<template>
  <div class="flex flex-wrap items-start gap-8">
    <figure v-for="(state, index) in states" :key="state.label" class="flex flex-col gap-2">
      <SelectionField
        v-if="shown[index]"
        v-model="state.message"
        :files="page.files"
        ask
        send
        :way="state.way"
        :held="state.held"
        :unavailable="state.unavailable ? t('comments.waitReply') : null"
        @comment="productWould('Add the Words as a Comment on the Selection')"
        @ask="productWould('Ask the Words as a Side Question About the Selection')"
        @send="productWould('Send the Quote and the Words to the Main Agent Now')"
        @close="close(index)"
      />
      <figcaption class="text-[12px] text-fg-subtle">{{ state.label }}</figcaption>
    </figure>
  </div>
</template>

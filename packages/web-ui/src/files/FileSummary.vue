<script setup lang="ts">
import { computed } from 'vue'
import FileCard from './FileCard.vue'
import { baseName } from '@demicodes/utils'
import { TOO_LARGE_NOTE } from './preview'
import { useShowing } from './showing'
import type { FileContents } from './types'

/**
 * The card of a file the view does not show, with what `contents` can say
 * about it and Download; without `contents`, its name alone. A file its
 * source is too large to serve says so and offers no Download. The facts
 * follow the file as its source keeps it.
 */
const props = defineProps<{
  path: string
  contents?: FileContents
  note?: string | null
  /** The note for a file too large for its source to serve. */
  tooLarge?: string
}>()

const shown = useShowing(() => props.contents, () => props.path, (contents, path) => contents.showDescription(path))
// Any failure but size leaves the card without the facts; its name and Download still hold.
const tooLarge = computed(() => shown.entry.value?.failure?.kind === 'too-large')
const description = computed(() => tooLarge.value ? null : shown.entry.value?.value ?? null)

const download = computed(() => tooLarge.value
  ? null
  : props.contents?.url(props.path, { download: true }) ?? null)
</script>

<template>
  <FileCard
    :name="baseName(path)"
    :description="description"
    :note="tooLarge ? props.tooLarge ?? TOO_LARGE_NOTE : note"
    :download="download"
  />
</template>

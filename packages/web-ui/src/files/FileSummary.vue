<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import FileCard from './FileCard.vue'
import { baseName } from './paths'
import { TOO_LARGE_NOTE } from './preview'
import { FileBrowserError, type FileContents, type FileDescription } from './types'

/**
 * The card of a file the view does not show, with what `contents` can say
 * about it and Download; without `contents`, its name alone. A file its
 * source is too large to serve says so and offers no Download.
 */
const props = defineProps<{
  path: string
  contents?: FileContents
  note?: string | null
  /** The note for a file too large for its source to serve. */
  tooLarge?: string
}>()

type Facts =
  | { phase: 'unknown' }
  | { phase: 'described'; description: FileDescription }
  | { phase: 'too-large' }

const facts = ref<Facts>({ phase: 'unknown' })
let controller: AbortController | null = null

async function describe(): Promise<void> {
  controller?.abort()
  facts.value = { phase: 'unknown' }
  if (!props.contents)
    return
  const current = new AbortController()
  controller = current
  try {
    const description = await props.contents.describe(props.path, current.signal)
    if (!current.signal.aborted)
      facts.value = { phase: 'described', description }
  } catch (error) {
    if (!current.signal.aborted && error instanceof FileBrowserError && error.kind === 'too-large')
      facts.value = { phase: 'too-large' }
    // Any other failure leaves the card without the facts; its name and Download still hold.
  }
}

const download = computed(() => facts.value.phase === 'too-large'
  ? null
  : props.contents?.url(props.path, { download: true }) ?? null)

watch(() => [props.path, props.contents], describe, { immediate: true })

onBeforeUnmount(() => {
  controller?.abort()
})
</script>

<template>
  <FileCard
    :name="baseName(path)"
    :description="facts.phase === 'described' ? facts.description : null"
    :note="facts.phase === 'too-large' ? tooLarge ?? TOO_LARGE_NOTE : note"
    :download="download"
  />
</template>

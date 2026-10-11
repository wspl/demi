<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import RegionStatus from '../ui/RegionStatus.vue'
import FileCard from './FileCard.vue'
import ImagePreview from './ImagePreview.vue'
import MediaPreview from './MediaPreview.vue'
import { baseName } from '@demicodes/utils'
import { mediaFactParts, type MediaFacts, type ShownFile } from './file-bar'
import { MISSING_FILE, TOO_LARGE_NOTE } from './preview'
import { useShowing } from './showing'
import type { FileContents, FileDescription } from './types'

/**
 * An image, a video, an audio file or a PDF, loaded from `contents` in the
 * web browser's own viewer (`file-previews.md` § What the user sees). The
 * preview pins the version it shows, so it never shows a mix of two; when
 * the file's description is read again with a new version, the new one
 * replaces it in place. A viewer that cannot show its bytes has the file
 * described again: a new version shows, and the same one is a file this
 * browser cannot show. Its pixel size, an image's zoom and its file size
 * stand under it, unless its host shows them elsewhere, as the File view's
 * bar does from what the preview reports (`shown`).
 */
const props = withDefaults(defineProps<{
  path: string
  kind: 'image' | 'video' | 'audio' | 'pdf'
  contents: FileContents
  /** The card's note for a file too large for its source to serve. */
  tooLarge?: string
  /** Whether the facts stand under the file. */
  caption?: boolean
}>(), { tooLarge: undefined, caption: true })

const emit = defineEmits<{
  /** What shows of the file, as it changes, an image's zoom included. */
  shown: [shown: ShownFile]
}>()

type State =
  | { phase: 'loading' }
  | { phase: 'ready'; description: FileDescription }
  | { phase: 'undecodable'; description: FileDescription }
  | { phase: 'too-large' }
  /** Nothing is at the path. */
  | { phase: 'missing' }
  | { phase: 'failed'; message: string }

const dimensions = ref<{ width: number; height: number } | null>(null)
/** An image's zoom, which its preview reports. */
const zoom = ref<number | null>(null)
const name = computed(() => baseName(props.path))
const shown = useShowing(() => props.contents, () => props.path, (contents, path) => contents.showDescription(path))
/** The version the web browser could not decode, of this file. */
const undecodable = ref<{ path: string; version: string | null } | null>(null)

const state = computed<State>(() => {
  const entry = shown.entry.value
  const description = entry?.value
  if (!entry || (entry.answer === null && entry.failure === null))
    return { phase: 'loading' }
  if (entry.answer === 'too-large')
    return { phase: 'too-large' }
  if (entry.answer === 'gone')
    return { phase: 'missing' }
  if (entry.answer !== 'value' || description === undefined)
    return { phase: 'failed', message: entry.failure?.message ?? 'The file could not be read.' }
  const failed = undecodable.value
  return failed?.path === props.path && failed.version === description.version
    ? { phase: 'undecodable', description }
    : { phase: 'ready', description }
})

/** A viewer that cannot show the bytes: the file changed, or the web browser cannot decode it. */
function failed(): void {
  if (state.value.phase !== 'ready')
    return
  undecodable.value = { path: props.path, version: state.value.description.version }
  shown.retry()
}

const src = computed(() => state.value.phase === 'ready'
  ? props.contents.url(props.path, state.value.description.version === null ? {} : { version: state.value.description.version })
  : '')

watch(() => props.path, () => {
  dimensions.value = null
  zoom.value = null
})

/** What the preview knows of the file it shows: its pixel size, an image's zoom, and its file size. */
const facts = computed<MediaFacts | null>(() => state.value.phase === 'ready'
  ? { pixels: dimensions.value, zoom: zoom.value, size: state.value.description.size }
  : null)

const shownFile = computed<ShownFile>(() => {
  const current = state.value
  if (facts.value)
    return { as: 'media', facts: facts.value }
  switch (current.phase) {
    case 'undecodable':
      return { as: 'card', size: current.description.size }
    case 'too-large':
      return { as: 'card', size: null }
    case 'missing':
      return { as: 'missing' }
    default:
      return { as: 'reading' }
  }
})
watch(shownFile, (shown) => emit('shown', shown), { immediate: true })
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <template v-if="state.phase === 'ready'">
      <div class="min-h-0 flex-1">
        <ImagePreview
          v-if="kind === 'image'"
          :src="src"
          :name="name"
          @size="(width, height) => dimensions = { width, height }"
          @zoom="zoom = $event"
          @failed="failed"
        />
        <MediaPreview
          v-else-if="kind === 'video' || kind === 'audio'"
          :src="src"
          :kind="kind"
          :name="name"
          @size="(width, height) => dimensions = { width, height }"
          @failed="failed"
        />
        <!-- The web browser's PDF viewer refuses a sandboxed frame. -->
        <iframe v-else :src="src" :title="name" class="h-full w-full border-0" />
      </div>
      <div v-if="caption && facts" class="shrink-0 px-3 pb-2 text-center text-[11px] tabular-nums text-fg-muted">
        {{ mediaFactParts(facts).join(' · ') }}
      </div>
    </template>
    <FileCard
      v-else-if="state.phase === 'undecodable'"
      :name="name"
      :description="state.description"
      note="This browser cannot show this file."
      :download="contents.url(path, { download: true })"
    />
    <FileCard
      v-else-if="state.phase === 'too-large'"
      :name="name"
      :description="null"
      :note="tooLarge ?? TOO_LARGE_NOTE"
    />
    <RegionStatus
      v-else
      class="h-full"
      :status="state.phase === 'loading' ? 'loading' : 'failed'"
      :icon="state.phase === 'missing' ? MISSING_FILE.icon : undefined"
      :label="state.phase === 'loading' ? 'Reading…' : state.phase === 'missing' ? MISSING_FILE.label : 'Could not read this file.'"
      loading-label="Reading…"
      :detail="state.phase === 'failed' ? state.message : null"
      :on-retry="shown.retry"
    />
  </div>
</template>

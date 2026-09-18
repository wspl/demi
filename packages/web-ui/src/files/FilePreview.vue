<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import RegionStatus from '../ui/RegionStatus.vue'
import FileCard from './FileCard.vue'
import ImagePreview from './ImagePreview.vue'
import MediaPreview from './MediaPreview.vue'
import { baseName } from './paths'
import { formatBytes } from './format'
import { TOO_LARGE_NOTE } from './preview'
import { FileBrowserError, type FileContents, type FileDescription } from './types'

/**
 * An image, a video, an audio file or a PDF, loaded from `contents` in the
 * browser's own viewer (`file-previews.md` § What the user sees). The
 * preview pins the version it opened: when the file changes under it, it
 * says so and offers the new one instead of showing a mix of both.
 */
const props = defineProps<{
  path: string
  kind: 'image' | 'video' | 'audio' | 'pdf'
  contents: FileContents
  /** The card's note for a file too large for its source to serve. */
  tooLarge?: string
}>()

type State =
  | { phase: 'loading' }
  | { phase: 'ready'; description: FileDescription }
  | { phase: 'undecodable'; description: FileDescription }
  | { phase: 'changed' }
  | { phase: 'too-large' }
  | { phase: 'failed'; message: string }

const state = ref<State>({ phase: 'loading' })
const dimensions = ref<{ width: number; height: number } | null>(null)
const name = computed(() => baseName(props.path))
let controller: AbortController | null = null

async function describe(): Promise<FileDescription | null> {
  controller?.abort()
  const current = new AbortController()
  controller = current
  try {
    return await props.contents.describe(props.path, current.signal)
  } catch (error) {
    if (current.signal.aborted)
      return null
    state.value = error instanceof FileBrowserError && error.kind === 'too-large'
      ? { phase: 'too-large' }
      : { phase: 'failed', message: error instanceof Error ? error.message : String(error) }
    return null
  }
}

async function open(): Promise<void> {
  state.value = { phase: 'loading' }
  dimensions.value = null
  const description = await describe()
  if (description)
    state.value = { phase: 'ready', description }
}

/** A viewer that cannot show the bytes: the file changed, or the browser cannot decode it. */
async function failed(): Promise<void> {
  if (state.value.phase !== 'ready')
    return
  const opened = state.value.description
  const now = await describe()
  if (!now)
    return
  state.value = now.version !== opened.version
    ? { phase: 'changed' }
    : { phase: 'undecodable', description: now }
}

const src = computed(() => state.value.phase === 'ready'
  ? props.contents.url(props.path, state.value.description.version === null ? {} : { version: state.value.description.version })
  : '')

const caption = computed(() => {
  if (state.value.phase !== 'ready')
    return ''
  const size = formatBytes(state.value.description.size)
  return dimensions.value ? `${dimensions.value.width} × ${dimensions.value.height} · ${size}` : size
})

watch(() => [props.path, props.contents], open, { immediate: true })

onBeforeUnmount(() => {
  controller?.abort()
})
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
        <!-- The browser's PDF viewer refuses a sandboxed frame. -->
        <iframe v-else :src="src" :title="name" class="h-full w-full border-0" />
      </div>
      <div class="shrink-0 px-3 pb-2 text-center text-[11px] tabular-nums text-fg-muted">{{ caption }}</div>
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
      v-else-if="state.phase === 'changed'"
      class="h-full"
      label="The file changed."
      action="Show the new version"
      @action="open"
    />
    <RegionStatus
      v-else
      class="h-full"
      :busy="state.phase === 'loading'"
      :failed="state.phase === 'failed'"
      :label="state.phase === 'loading' ? 'Reading…' : 'Could not read this file.'"
      :detail="state.phase === 'failed' ? state.message : null"
      :action="state.phase === 'failed' ? 'Retry' : undefined"
      @action="open"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { HTML_SANDBOX } from '@demicodes/protocol'
import { baseName } from '@demicodes/utils'
import { usePaneReady } from '../agent/pane-ready'
import RegionStatus from '../ui/RegionStatus.vue'
import { t } from './locales/text'
import type { FileContents } from './types'

/**
 * An HTML file as the page it describes, in a frame sandboxed with
 * `HTML_SANDBOX`: an opaque origin, so its scripts reach nothing of the
 * product's, and its stylesheets, scripts and images load from beside it.
 *
 * - It draws on white, as a web browser draws a page without a background,
 *   whatever the theme.
 * - Each new version of the file loads the page again.
 * - Where the page's URL cannot be had, it says so, with Retry.
 */
const props = defineProps<{
  path: string
  /** The version of the file's text shown; a new one loads the page again. */
  version: string | null
  contents: FileContents & Required<Pick<FileContents, 'htmlUrl'>>
}>()

type State =
  | { phase: 'loading' }
  /** The URL asked for at `version`. */
  | { phase: 'ready'; url: string; version: string | null }
  | { phase: 'failed'; message: string; requestId: string | null }

const state = ref<State>({ phase: 'loading' })
/** The ask answered last, so an answer for a file left behind is dropped. */
let asked = 0

async function load(): Promise<void> {
  const ask = ++asked
  const { path, version } = props
  try {
    const url = await props.contents.htmlUrl(path)
    if (ask === asked)
      state.value = { phase: 'ready', url, version }
  } catch (error) {
    if (ask !== asked)
      return
    const failure = error as { message?: string; requestId?: string | null }
    state.value = { phase: 'failed', message: failure.message ?? t('view.readFailedDetail'), requestId: failure.requestId ?? null }
  }
}

// The URL is asked for again for each version: a source may answer one that
// holds the text, and the token in the backend's lives only as long as its
// process. A new version keeps the page shown until its URL is in.
watch(() => props.path, () => {
  state.value = { phase: 'loading' }
})
watch(() => [props.path, props.version], () => void load(), { immediate: true })

/** The frame is made again for each version, which loads the page again, at the same URL too. */
const frameKey = computed(() => state.value.phase === 'ready' ? `${state.value.url}\u0000${state.value.version ?? ''}` : '')

usePaneReady(() => state.value.phase !== 'loading')
</script>

<template>
  <iframe
    v-if="state.phase === 'ready'"
    :key="frameKey"
    class="block h-full w-full border-0 bg-white"
    :src="state.url"
    :sandbox="HTML_SANDBOX"
    :title="baseName(path)"
  />
  <RegionStatus
    v-else
    class="h-full"
    :status="state.phase"
    :label="state.phase === 'loading' ? t('reading') : t('view.readFailed')"
    :loading-label="t('reading')"
    :detail="state.phase === 'failed' ? state.message : null"
    :request-id="state.phase === 'failed' ? state.requestId : null"
    :on-retry="load"
  />
</template>

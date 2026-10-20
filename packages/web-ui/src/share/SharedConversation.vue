<script setup lang="ts">
import { computed, ref } from 'vue'
import type { HeldTranscript } from '@demicodes/conversation-client'
import AgentMessageList from '../agent/AgentMessageList.vue'
import SessionSurface from '../agent/SessionSurface.vue'
import type { ShownError } from '../infra/errors'
import type { ListLoad } from '../agent/session-status'
import { shownWindow, windowEdges } from '../agent/history'
import { provideMessageFiles } from '../markdown/message-files'
import type { AttachmentLookup } from '../markdown/types'
import { sharedRows, type SharedItem } from './shared-items'
import { t } from './locales/text'

/**
 * A share's read-only page: what was said in the conversation, its messages and
 * texts, with each run of the agent's work between them as one line that counts
 * its steps, under its title and with nothing that acts on a conversation. A
 * link to a Host's file, a commit or an agent is its text, since the page
 * reaches neither a Host nor the work.
 */
const props = withDefaults(defineProps<{
  /** The conversation's title when shared; null while it was the placeholder. */
  title: string | null
  /** The parts of the share the page holds. */
  history: HeldTranscript<SharedItem>
  /** Each compaction marker's boundary's summary size, by the marker's id, as pages named them. */
  summaries?: Record<string, number>
  load: ListLoad
  loadError?: ShownError | null
  /** An attachment the share's text names, such as `a3`. */
  attachment: (id: string) => AttachmentLookup
}>(), {
  summaries: () => ({}),
  loadError: null,
})

const emit = defineEmits<{
  /** The reader nears the start of the window, which does not reach the share's. */
  readBefore: []
  /** The reader nears the end of the window, which does not reach the share's. */
  readAfter: []
  retryLoad: []
}>()

// No Host stands behind a share: its paths and commits stay text, its attachments show.
provideMessageFiles(() => ({ cwd: '', attachment: props.attachment }))

const shownTitle = computed(() => props.title?.trim() || t('share.untitled'))
// A share is read from its latest page back, as a subagent's panel reads it.
const shown = computed(() => shownWindow(props.history, null))
const edges = computed(() => windowEdges(props.history, shown.value))
const rows = computed(() => sharedRows(shown.value.blocks))
// The transcript reads in the conversation's one column, with no dock below it.
const surface = ref<InstanceType<typeof SessionSurface>>()
</script>

<template>
  <section class="flex h-full min-h-0 flex-col bg-surface" data-shared-conversation>
    <SessionSurface ref="surface" class="min-h-0">
      <div class="flex h-full flex-col">
        <!-- The title stands in the transcript's column, its text where the rows' text starts. -->
        <header class="flex h-11 shrink-0 items-center gap-1.5 px-[calc(var(--session-gutter)+var(--agent-pad-x,2rem))]">
          <h1 class="min-w-0 truncate text-chrome font-normal text-fg">{{ shownTitle }}</h1>
        </header>
        <AgentMessageList
          class="min-h-0 flex-1"
          conversation-id="share"
          :blocks="rows.blocks"
          :steps-not-shared="rows.steps"
          :at-start="edges.atStart"
          :at-end="edges.atEnd"
          :summaries="summaries"
          :pending-steers="[]"
          :queue="[]"
          phase="idle"
          :load="load"
          :load-error="loadError"
          :bottom-offset="surface?.dockHeight ?? 0"
          :persisted-scroll-state="undefined"
          read-only
          hide-fork
          @read-before="emit('readBefore')"
          @read-after="emit('readAfter')"
          @retry-load="emit('retryLoad')"
        />
      </div>
    </SessionSurface>
    <footer class="shrink-0 py-2 text-center text-detail text-fg-subtle">{{ t('share.footer') }}</footer>
  </section>
</template>

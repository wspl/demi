<script setup lang="ts">
import { computed, ref } from 'vue'
import { ChevronLeft } from '@lucide/vue'
import type { HeldTranscript } from '@demicodes/conversation-client'
import AgentMessageList from '../agent/AgentMessageList.vue'
import AgentMark from '../agent/AgentMark.vue'
import SessionSurface from '../agent/SessionSurface.vue'
import IconButton from '../ui/IconButton.vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { ShownError } from '../infra/errors'
import type { ListLoad } from '../agent/session-status'
import { shownWindow, windowEdges } from '../agent/history'
import { provideBlockReader, type BlockReader } from '../agent/whole-blocks'
import { provideMessageAgents } from '../agent/agent-links'
import { provideMessageFiles } from '../markdown/message-files'
import type { AttachmentLookup } from '../markdown/types'
import { sharedAgentName, sharedMessageAgents, type SharedAgent } from './shared-agents'
import { t } from './locales/text'

/**
 * A share's read-only page (`sharing.md`): the snapshot's transcript as the
 * conversation shows it, with its title above and nothing that acts on a
 * conversation. An `agent:` link shows that subagent's transcript in its
 * place, under a header that names it and goes back; a link to a Host's
 * file or a commit is its text, since the page reaches no Host.
 */
const props = withDefaults(defineProps<{
  /** The conversation's title when shared; null while it was the placeholder. */
  title: string | null
  agents: readonly SharedAgent[]
  /** The subagent whose transcript shows, by its number; null for the conversation's own. */
  agent: number | null
  /** The parts of the shown transcript the page holds (`web-application.md` § Transcript windows). */
  history: HeldTranscript
  /** Each compaction marker's boundary's summary size, by the marker's id, as pages named them. */
  summaries?: Record<string, number>
  load: ListLoad
  loadError?: ShownError | null
  /** Reads a block the page holds light whole. */
  readBlock: BlockReader
  /** An attachment the snapshot's text names, such as `a3`. */
  attachment: (id: string) => AttachmentLookup
}>(), {
  summaries: () => ({}),
  loadError: null,
})

const emit = defineEmits<{
  /** Show the transcript of subagent `number`, or the conversation's own for null. */
  openAgent: [number: number | null]
  /** The reader nears the start of the window, which does not reach the transcript's. */
  readBefore: []
  /** The reader nears the end of the window, which does not reach the transcript's. */
  readAfter: []
  retryLoad: []
}>()

provideBlockReader(() => props.readBlock)
// No Host stands behind a share: its paths and commits stay text, its attachments show.
provideMessageFiles(() => ({ cwd: '', attachment: props.attachment }))
provideMessageAgents(() => sharedMessageAgents(props.agents, (number) => emit('openAgent', number === 0 ? null : number)))

const shownTitle = computed(() => props.title?.trim() || t('share.untitled'))
const shownAgent = computed(() => props.agents.find((agent) => agent.number === props.agent) ?? null)
// A share is read from its latest page back, as a subagent's panel reads it.
const shown = computed(() => shownWindow(props.history, null))
const edges = computed(() => windowEdges(props.history, shown.value))
// The transcript reads in the conversation's one column, with no dock below it.
const surface = ref<InstanceType<typeof SessionSurface>>()
</script>

<template>
  <section class="flex h-full min-h-0 flex-col bg-surface" data-shared-conversation>
    <header class="flex h-11 shrink-0 items-center gap-1.5 px-3">
      <template v-if="agent !== null">
        <IconButton
          :icon="ChevronLeft"
          variant="ghost"
          :aria-label="t('share.back')"
          data-shared-back
          @click="emit('openAgent', null)"
        />
        <AgentMark :phase="null" :size="ICON_PX.in28" />
        <h1 class="min-w-0 truncate text-chrome font-normal text-fg">
          {{ shownAgent ? sharedAgentName(shownAgent) : '' }}
        </h1>
        <span v-if="shownAgent?.profile" class="shrink-0 truncate text-chrome text-fg-subtle">{{ shownAgent.profile }}</span>
      </template>
      <h1 v-else class="min-w-0 truncate text-chrome font-normal text-fg">{{ shownTitle }}</h1>
    </header>
    <SessionSurface ref="surface" class="min-h-0">
      <AgentMessageList
        :key="agent ?? 'root'"
        class="h-full"
        :conversation-id="`share:${agent ?? 'root'}`"
        :node="agent === null ? null : String(agent)"
        :blocks="shown.blocks"
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
    </SessionSurface>
    <footer class="shrink-0 py-2 text-center text-[12px] text-fg-subtle">{{ t('share.footer') }}</footer>
  </section>
</template>

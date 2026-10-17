<script setup lang="ts">
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import AssistantMessageFooter from './AssistantMessageFooter.vue'
import type { MessageForkState } from '../message-fork'
import type { AnswerShare } from '../sharing'
import type { VersionChoice } from '../versions'
import { useTextArrivals } from '../whole-blocks'
import { useCopy } from '../../composables/useCopy'
import { vContextMenu, type ContextMenuDeclaration } from '../../ui/context-menu'
import { answerActions, answerMenuEntries } from '../answer-actions'
import { quotedWords, sendEntry, useSendTargets } from '../send-to-conversation'

const props = withDefaults(defineProps<{
  content: string
  createdAt: string
  fork?: () => Promise<void>
  forkState?: MessageForkState
  share?: AnswerShare
  shareWait?: boolean
  hideFork?: boolean
  /** Regenerate in the footer; absent where it is not offered. */
  regenerate?: () => void
  /** The answer's versions in the footer; absent with none. */
  versions?: VersionChoice
  isStreaming?: boolean
  showFooter?: boolean
}>(), {
  isStreaming: false,
})

const arrivals = useTextArrivals()

const targets = useSendTargets()
const { copy } = useCopy(() => props.content)

/**
 * The answer's menu holds what its footer shows at that moment
 * (`answerActions`); text in the middle of a reply, which has no footer,
 * has none. Its text is the user's to select, and text selected in it goes
 * to another conversation as a quotation (`web-application.md` § Context menus).
 */
const menu: ContextMenuDeclaration = {
  entries: () => (props.showFooter && !props.isStreaming ? [answerMenuEntries(answerActions(props, copy))] : []),
  text: true,
  selectionEntries: (text) => (targets ? [sendEntry(targets, () => quotedWords(text))] : []),
}
</script>

<template>
  <div v-context-menu="menu" class="px-[var(--agent-pad-x,2rem)] py-1.5">
    <StreamedMarkdown
      :content="content"
      :streaming="isStreaming"
      :arrivals="arrivals()"
      class="text-conversation text-fg-body"
    />
    <AssistantMessageFooter
      v-if="showFooter && !isStreaming"
      :content="content"
      :created-at="createdAt"
      :fork="fork"
      :fork-state="forkState"
      :share="share"
      :share-wait="shareWait"
      :hide-fork="hideFork"
      :regenerate="regenerate"
      :versions="versions"
    />
  </div>
</template>

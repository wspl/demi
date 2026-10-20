<script setup lang="ts">
import { ref } from 'vue'
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
import { useReplySelectionTools } from '../reply-selection'
import { pictureWords, useKeepPicture } from '../comments/pictures'

const props = withDefaults(defineProps<{
  /** The text block the reply is, which its selection's tools act on; absent, they offer nothing. */
  blockId?: string
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
const tools = useReplySelectionTools()
const keepPicture = useKeepPicture()
const root = ref<HTMLElement>()
const { copy } = useCopy(() => props.content)

/**
 * The answer's menu holds what its footer shows at that moment
 * (`answerActions`); text in the middle of a reply, which has no footer,
 * has none. Its text is the user's to select, and text selected in it goes
 * to another conversation as a quotation.
 */
const menu: ContextMenuDeclaration = {
  entries: () => (props.showFooter && !props.isStreaming ? [answerMenuEntries(answerActions(props, copy))] : []),
  text: true,
  // The page's own tools act on the reply in place and come first; Send to Conversation, which opens a submenu, last.
  selectionEntries: (text) => [
    ...(tools && props.blockId && root.value ? tools.selection({ blockId: props.blockId, root: root.value }, () => props.isStreaming) : []),
    ...(targets ? [sendEntry(targets, () => quotedWords(text))] : []),
  ],
  // A picture takes a comment and a side question as words do, and goes to another conversation as a file.
  pictureEntries: (image) => [
    ...(tools && props.blockId && root.value ? tools.picture({ blockId: props.blockId, root: root.value }, image, () => props.isStreaming) : []),
    ...(targets ? [sendEntry(targets, () => pictureWords(image, keepPicture))] : []),
  ],
}
</script>

<template>
  <div ref="root" v-context-menu="menu" :data-reply-text="blockId" class="px-[var(--agent-pad-x,2rem)] py-1.5">
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

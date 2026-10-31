<script setup lang="ts">
import type { CodeFile, ReplyPicture } from '@demicodes/protocol'
import { computed, ref } from 'vue'
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
import { useSuggestedReplies } from '../suggested-replies'
import SuggestedReplies from './SuggestedReplies.vue'

const props = withDefaults(defineProps<{
  /**
   * The text block the reply is, which its selection's tools act on; absent,
   * they offer nothing.
   */
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
  /** The Host files the reply's inline code names, kept with it once it ended. */
  codeFiles?: readonly CodeFile[]
  /** The pictures of the Host the reply shows, with their copies, kept with it once it ended. */
  pictures?: readonly ReplyPicture[]
}>(), {
  isStreaming: false,
})

const arrivals = useTextArrivals()

const targets = useSendTargets()
const tools = useReplySelectionTools()
const keepPicture = useKeepPicture()
const root = ref<HTMLElement>()
const { copy } = useCopy(() => props.content)
const suggestions = useSuggestedReplies()
/** The suggested replies under this answer's footer, while it is the one they follow. */
const suggested = computed(() => {
  const now = suggestions?.current()
  return now && now.blockId === props.blockId ? now : null
})

/**
 * The answer's menu holds what its footer shows at that moment
 * (`answerActions`). Text in the middle of a reply, which has no footer, has no
 * menu. The text is the user's to select, and text selected in it goes to
 * another conversation as a quotation.
 */
const menu: ContextMenuDeclaration = {
  entries: () => (props.showFooter && !props.isStreaming ? [answerMenuEntries(answerActions(props, copy))] : []),
  text: true,
  // The page's own tools act on the reply in place and come first; Send to
  // Conversation, which opens a submenu, last.
  selectionEntries: (selected) => [
    ...(tools && props.blockId && root.value ? tools.selection({ blockId: props.blockId, root: root.value }, selected, () => props.isStreaming) : []),
    ...(targets ? [sendEntry(targets, () => quotedWords(selected.text))] : []),
  ],
  selectionField: (selected) =>
    tools && props.blockId && root.value ? tools.field({ blockId: props.blockId, root: root.value }, selected, () => props.isStreaming) : null,
  // A picture takes a comment and a side question as words do, and goes to
  // another conversation as a file.
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
      :code-files="codeFiles"
      :pictures="pictures"
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
    <SuggestedReplies
      v-if="suggested && suggestions && showFooter && !isStreaming"
      :replies="suggested.replies"
      @send="suggestions.send"
      @edit="suggestions.edit"
    />
  </div>
</template>

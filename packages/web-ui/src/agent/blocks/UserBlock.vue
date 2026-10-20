<script setup lang="ts">
import { t } from '../locales/text'
import { t as shared } from '../../locales/text'
import { computed, ref } from 'vue'
import type { Component } from 'vue'
import { useElementSize, useResizeObserver } from '@vueuse/core'
import { Copy, Pencil, Trash2, Zap } from '@lucide/vue'
import { useCopy } from '../../composables/useCopy'
import { vContextMenu, type ContextMenuDeclaration } from '../../ui/context-menu'
import { sendEntry, textWords, useSendTargets } from '../send-to-conversation'
import { contentCopy, type CopiedMessage } from '../message-input/copied-message'
import { commentsContent } from '../comments/comments'
import { messageSchema } from '../message-editor/extensions'
import { useMessageClipboard } from '../message-editor/message-clipboard'
import { parseUserMarkdown } from '../../markdown/user-markdown'
import Button from '@demicodes/web-ui/ui/Button.vue'
import CopyButton from '@demicodes/web-ui/ui/CopyButton.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import type { CopySource } from '../../composables/useCopy'
import type { MessageEditContent } from '../message-editing'
import type { VersionChoice } from '../versions'
import type { MessageOrigin } from '../message-origins'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import NoticeLabel from './NoticeLabel.vue'
import VersionSwitcher from './VersionSwitcher.vue'
import { composerCapsule, composerTransfer, contentMessage, provideTransfers } from '../message-editor/capsules'
import SentMessage from '../message-editor/SentMessage.vue'
import StreamedMarkdown from '../../ui/StreamedMarkdown.vue'
import type { ComposerAttachment } from '../message-input/attachments'
import type { MediaSource } from '@demicodes/protocol'
import { useMediaAddress } from '../media-source'

const props = defineProps<{
  content: readonly MessageEditContent[]
  /**
   * The files of a message the server has not confirmed yet, in the order of
   * the attachment marks in its text; its content is then text alone.
   */
  attachments?: readonly ComposerAttachment[]
  forceStuck?: boolean
  /**
   * A message the agent has not read yet, drawn faded, with what happens to it
   * said under it and its controls in view. It is one of two kinds:
   *
   * - `steer`: the running turn reads it at its next step.
   * - `queued`: it runs after the turn.
   *
   * Send Now delivers either at once. Recall takes it back, into the composer
   * when that is empty.
   */
  pending?: 'steer' | 'queued'
  /**
   * The comments sent with the message: each quoted text as a quote block
   * followed by its comment, before the message's own text; a picture's as a
   * small copy of it.
   */
  comments?: readonly { id: string; quote: string; picture?: MediaSource | null; text: string }[]
  /** Keep the hover actions visible (a catalog specimen, not a hover). */
  actionsPinned?: boolean
  /** Allows editing an explicitly submitted user message. */
  editable?: boolean
  /**
   * Offers Delete beside Edit, for a message that was not delivered. Delete
   * takes the message away at once.
   */
  deletable?: boolean
  /** The message's versions, shown under it; absent with none. */
  versions?: VersionChoice
  /**
   * A child's task brief (`childBrief`): rendered as an answer is, in full
   * Markdown, inside the same bubble, and copied as its Markdown; never
   * editable.
   */
  brief?: string | null
  /**
   * What sent the message when the user did not write it, an automation: named
   * above the bubble, as a receipt names the agent that sent one.
   */
  origin?: MessageOrigin | null
}>()

/**
 * Where a picture comment's picture loads from, which the bubble shows as its
 * quote.
 */
const mediaAddress = useMediaAddress()


const emit = defineEmits<{
  recall: []
  sendNow: []
  /** Open this message's editor. */
  edit: []
  /** Delete the message, which was not delivered. */
  delete: []
}>()

const sent = ref<InstanceType<typeof SentMessage>>()

const clipboard = useMessageClipboard()

/**
 * Copy puts the message on the clipboard in every form, its files with it, its
 * comments before its text as the model reads them. A brief copies as its
 * Markdown, as an answer's Copy does.
 */
function copySource(): CopySource {
  if (typeof props.brief === 'string') {
    return props.brief
  }
  if (!props.comments?.length) {
    return sent.value?.clipboardItem()
  }
  const schema = messageSchema()
  // The comments' marks come first, as their pictures' files do.
  const lead = contentMessage(commentsContent(props.comments))
  const { markdown, capsules } = message.value
  const text = lead.markdown && markdown ? `${lead.markdown}\n\n${markdown}` : lead.markdown || markdown
  const doc = schema.nodeFromJSON(parseUserMarkdown(text, [...lead.capsules, ...capsules]))
  return clipboard.item(clipboard.forms(schema, doc.content))
}

/**
 * The message as Send to Conversation takes it: its comments before its text,
 * their pictures as files with its own.
 */
function sentWords(): CopiedMessage {
  return contentCopy([...commentsContent(props.comments ?? []), ...props.content])
}

// A message not yet confirmed shows how far each of its files' uploads has
// come, and that one failed; the dock's Retry tries the message again.
provideTransfers({
  transfer: (id) => {
    const item = props.attachments?.find((each) => each.id === id)
    return item ? composerTransfer(item) : undefined
  },
  carries: () => false,
  current: (id) => {
    const item = props.attachments?.find((each) => each.id === id)
    return item ? composerCapsule(item) : undefined
  },
})

interface BubbleAction {
  key: string
  hint: string
  icon: Component
  emit: () => void
}

/**
 * The message as it shows: its Markdown, and a capsule for each file in the
 * order of their marks.
 */
const message = computed(() => {
  const held = contentMessage(props.content)
  return props.attachments ? { ...held, capsules: props.attachments.map(composerCapsule) } : held
})

// A sent message gets Copy and, where it can be edited, Edit on hover; one
// not delivered, Delete too.
const actions = computed<BubbleAction[]>(() => {
  if (!props.editable || typeof props.brief === 'string') {
    return []
  }
  return [
    { key: 'edit', hint: t('transcript.user.edit'), icon: Pencil, emit: () => emit('edit') },
    ...(props.deletable ? [{ key: 'delete', hint: shared('common.delete'), icon: Trash2, emit: () => emit('delete') }] : []),
  ]
})

const { copy } = useCopy(copySource)

/**
 * The bubble's widest width: four fifths of the row. It is narrower where its
 * actions, which sit left of the bubble, would not fit beside it otherwise, as
 * three do on a phone.
 */
const actionsRef = ref<HTMLElement | null>(null)
const { width: actionsWidth } = useElementSize(actionsRef, undefined, { box: 'border-box' })
const bubbleWidth = computed(() => ({
  maxWidth: `min(80%, calc(100% - ${actionsWidth.value}px - 6px))`,
}))

const targets = useSendTargets()
/**
 * Whether the message is the user's own, which Send to Conversation takes
 * elsewhere; a child's brief is not.
 */
const own = computed(() => typeof props.brief !== 'string')

/**
 * The message's menu: what its controls offer, Copy and, where it can be
 * edited, Edit, then Send to Conversation, which takes it with its files. A
 * message not yet read has its Send Now and Recall.
 */
const menu: ContextMenuDeclaration = {
  entries: () => {
    if (props.pending) {
      return [[
        { label: t('transcript.user.sendNow'), run: () => emit('sendNow') },
        { label: t('transcript.user.recall'), run: () => emit('recall') },
      ]]
    }
    return [[
      { icon: Copy, label: t('transcript.copyMessage'), run: copy },
      ...actions.value.map((action) => ({ icon: action.icon, label: action.hint, run: action.emit })),
      ...(targets && own.value ? [sendEntry(targets, sentWords)] : []),
    ]]
  },
  selectionEntries: (text) => (targets && own.value && !props.pending ? [sendEntry(targets, () => textWords(text))] : []),
  text: true,
}

const textClass = computed(() => (props.pending ? 'text-fg-subtle' : 'text-fg-body'))

/**
 * A message taller than `FOLD_ABOVE_PX` folds to show about `FOLDED_PX` of it,
 * its last line fading out, until Show More opens it, as Claude Code's does. A
 * message a few lines over the fold shows whole, since folding would hide less
 * than the control takes.
 */
const FOLD_ABOVE_PX = 300
const FOLDED_PX = 200
const expanded = ref(false)

const contentRef = ref<HTMLElement>()
const bodyRef = ref<HTMLElement>()
/**
 * Where a long message folds, and the line box its fade spans; null when it is
 * short enough to show whole. The fold is `FOLDED_PX` down, or lower, at the
 * end of a line of text across that mark, so it cuts no letter. Anything else
 * across it, such as an image or a code block, is cut there.
 */
const clip = ref<{ height: number; line: number } | null>(null)

function measure(): void {
  const box = contentRef.value
  const body = bodyRef.value
  if (!box || !body) {
    clip.value = null
    return
  }
  const line = Number.parseFloat(getComputedStyle(body).lineHeight) || 0
  const limit = FOLDED_PX
  const full = body.getBoundingClientRect().height
  if (full <= FOLD_ABOVE_PX) {
    clip.value = null
    return
  }
  const top = box.getBoundingClientRect().top
  // A glyph box sits in its line box a little off centre, so a line crosses
  // the mark only with a quarter of it on each side.
  const slack = line / 4
  let height = limit
  const texts = document.createTreeWalker(body, NodeFilter.SHOW_TEXT)
  const range = document.createRange()
  for (let text = texts.nextNode(); text; text = texts.nextNode()) {
    range.selectNodeContents(text)
    const rects = [...range.getClientRects()]
    // Text runs top to bottom: once it starts below the mark, the rest does
    // too.
    if (rects[0] && rects[0].top - top > limit) {
      break
    }
    for (const rect of rects) {
      // A glyph box is shorter than its line; take the line box around it.
      const half = Math.max(0, (line - rect.height) / 2)
      const lineTop = rect.top - top - half
      const lineBottom = rect.bottom - top + half
      if (lineTop < limit - slack && lineBottom > limit + slack) {
        height = Math.max(height, Math.ceil(lineBottom))
      }
    }
  }
  clip.value = height < full - 1 ? { height, line } : null
}

// Wrapping follows the width, so a narrower or wider column changes the cut.
// The text alone is watched: it takes the box's width, while the box's height
// is the cut itself, which would call the measure back for nothing.
useResizeObserver(bodyRef, measure)
</script>

<template>
  <div
    v-context-menu="menu"
    class="group/user relative z-10 flex flex-col items-end bg-surface px-[var(--agent-pad-x,2rem)] pb-2 pt-1.5"
    :class="forceStuck ? 'user-sticky' : ''"
  >
    <div v-if="origin" class="flex max-w-[80%] select-none items-center gap-1.5 pb-1 text-detail text-fg-subtle">
      <Zap :size="ICON_PX.in24" class="shrink-0" aria-hidden="true" />
      <NoticeLabel :notice="t('transcript.user.fromAutomation')" :title="origin.name" :go="origin.go" />
      <span class="shrink-0">· {{ origin.when }}</span>
    </div>
    <div class="relative rounded-xl bg-surface-card px-4 py-2.5" :style="bubbleWidth">
      <div
        v-if="!pending"
        ref="actionsRef"
        class="user-actions absolute bottom-2.5 left-0 flex -translate-x-[calc(100%+6px)] items-center transition-opacity group-hover/user:opacity-100 focus-within:opacity-100 phone:opacity-100"
        :class="actionsPinned ? 'opacity-100' : 'opacity-0'"
      >
        <CopyButton class="inline-flex" :source="copySource" @click.stop />
        <Tooltip
          v-for="action in actions"
          :key="action.key"
          :content="action.hint"
          class="inline-flex"
        >
          <IconButton
            :icon="action.icon"
            size="sm"
            variant="ghost"
            :aria-label="action.hint"
            @click.stop="action.emit()"
          />
        </Tooltip>
      </div>
      <div
        ref="contentRef"
        class="overflow-hidden"
        :style="
          clip && !expanded
            ? {
                height: `${clip.height}px`,
                maskImage: `linear-gradient(to bottom, black calc(100% - ${clip.line}px), transparent)`,
              }
            : undefined
        "
      >
        <div
          ref="bodyRef"
          class="select-text text-conversation"
          :class="textClass"
        >
          <StreamedMarkdown
            v-if="typeof brief === 'string'"
            :content="brief"
          />
          <template v-else>
            <!--
              The quoted text is the reply's words as they rendered, never read
              as Markdown again.
            -->
            <div v-if="comments?.length" class="markdown-body" :class="message.markdown || message.capsules.length ? 'mb-[var(--markdown-block-gap)]' : ''" data-sent-comments>
              <template v-for="comment in comments" :key="comment.id">
                <blockquote v-if="comment.picture" data-comment-picture>
                  <img :src="mediaAddress(comment.picture)" :alt="comment.quote" class="max-h-24 max-w-40 rounded-md object-contain">
                </blockquote>
                <blockquote v-else><p class="whitespace-pre-wrap">{{ comment.quote }}</p></blockquote>
                <p class="whitespace-pre-wrap">{{ comment.text }}</p>
              </template>
            </div>
            <SentMessage
              v-if="message.markdown || message.capsules.length || !comments?.length"
              ref="sent"
              :markdown="message.markdown"
              :attachments="message.capsules"
            />
          </template>
        </div>
      </div>
      <!-- Under the text, its label in line with the text's start. -->
      <Button
        v-if="clip"
        size="sm"
        variant="ghost"
        class="-ml-2 mt-1"
        :aria-expanded="expanded"
        @click.stop="expanded = !expanded"
      >{{ expanded ? t('transcript.user.showLess') : t('transcript.user.showMore') }}</Button>
    </div>
    <!-- An edited message's versions, under it at its end, always in view. -->
    <VersionSwitcher v-if="versions && !pending" :choice="versions" class="-mr-1 mt-0.5" />
    <!--
      A pending message says what happens to it, its controls in view rather
      than on hover.
    -->
    <div
      v-if="pending"
      class="mt-1 flex max-w-[80%] select-none flex-wrap items-center justify-end gap-x-1 text-detail leading-5 text-fg-subtle"
    >
      <!--
        A queued message stands under the queue's divider, which already says
        so.
      -->
      <span v-if="pending === 'steer'" class="mr-1">{{ t('transcript.user.sendsNext') }}</span>
      <Button size="xs" variant="ghost" @click.stop="emit('sendNow')">{{ t('transcript.user.sendNow') }}</Button>
      <Button size="xs" variant="ghost" @click.stop="emit('recall')">{{ t('transcript.user.recall') }}</Button>
    </div>
  </div>
</template>

<style scoped>
/*
 * Sits on the last text line: the bubble's bottom padding plus one line box,
 * buttons centered in it.
 */
.user-actions {
  height: calc(var(--agent-text) * var(--agent-leading));
}

.user-sticky::after {
  content: '';
  position: absolute;
  bottom: -32px;
  inset-inline: 0;
  height: 33px;
  background: linear-gradient(to bottom, var(--color-surface), transparent);
  pointer-events: none;
}
</style>

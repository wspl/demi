<script setup lang="ts">
import { computed, ref } from 'vue'
import type { Component } from 'vue'
import { useClipboard, useResizeObserver } from '@vueuse/core'
import { ArrowUp, Check, ChevronsUp, Copy, Pencil, X } from '@lucide/vue'
import type { UserContentBlock } from '@demicodes/core'
import { md } from '@demicodes/web-ui/markdown/md'
import { openFileLink, useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import AttachmentTile from '../AttachmentTile.vue'
import ContentMedia from '../ContentMedia.vue'
import {
  attachmentCaption,
  contentBlockCaption,
  decodeRemoteReference,
  type ComposerAttachment,
} from '../message-input/attachments'

const props = defineProps<{
  content: UserContentBlock[]
  /** Files still on their way to the transcript (an unconfirmed message), shown among the media. */
  attachments?: readonly ComposerAttachment[]
  forceStuck?: boolean
  pending?: boolean
  deletable?: boolean
  sendable?: boolean
  interruptible?: boolean
  /** Keep the hover actions visible (a catalog specimen, not a hover). */
  actionsPinned?: boolean
  /** Allows editing an explicitly submitted user message. */
  editable?: boolean
}>()

const emit = defineEmits<{
  delete: []
  sendNow: []
  interrupt: []
  /** Open this message's editor. */
  edit: []
}>()

const { copy, copied } = useClipboard({ copiedDuring: 1500 })

interface BubbleAction {
  key: string
  hint: string
  icon: Component
  emit: () => void
}

const userText = computed(() => props.content.flatMap(
  (part) => part.type === 'text' ? [part.text] : [],
).join('\n\n'))

// Sent messages get copy and edit; pending ones get the queue and steer controls instead.
const actions = computed<BubbleAction[]>(() => {
  const list: BubbleAction[] = []
  if (!props.pending) {
    list.push({
      key: 'copy',
      hint: copied.value ? 'Copied' : 'Copy',
      icon: copied.value ? Check : Copy,
      emit: () => void copy(userText.value),
    })
    if (props.editable) {
      list.push({
        key: 'edit',
        hint: 'Edit',
        icon: Pencil,
        emit: () => emit('edit'),
      })
    }
  }
  if (props.deletable) {
    list.push({
      key: 'delete',
      hint: props.sendable ? 'Remove' : 'Discard',
      icon: X,
      emit: () => emit('delete'),
    })
  }
  if (props.sendable) {
    list.push({
      key: 'send',
      hint: 'Send now',
      icon: ArrowUp,
      emit: () => emit('sendNow'),
    })
  }
  if (props.interruptible) {
    list.push({
      key: 'interrupt',
      hint: 'Interrupt and send',
      icon: ChevronsUp,
      emit: () => emit('interrupt'),
    })
  }
  return list
})

type MediaBlock = Extract<UserContentBlock, { type: 'image' | 'video' | 'document' }>
type AttachmentBlock = Extract<UserContentBlock, { type: 'attachment' }>

/**
 * One tile per file the message carried. A file the model reads natively has
 * its media block right before its attachment block; that media is the tile
 * (the picture itself). Every other attachment is a plain tile with its name
 * and, for text, its opening lines. A media block with no attachment after it
 * (a message that carried no record) is shown as it is.
 */
const fileTiles = computed(() => {
  const tiles: { key: string; file: AttachmentBlock | null; media: MediaBlock | null }[] = []
  props.content.forEach((block, index) => {
    if (block.type === 'attachment') {
      const previous = props.content[index - 1]
      const media = previous && isMediaBlock(previous) ? previous : null
      tiles.push({ key: block.path, file: block, media })
      return
    }
    if (isMediaBlock(block) && props.content[index + 1]?.type !== 'attachment') {
      tiles.push({ key: `media-${index}`, file: null, media: block })
    }
  })
  return tiles
})

function isMediaBlock(block: UserContentBlock): block is MediaBlock {
  return block.type === 'image' || block.type === 'video' || block.type === 'document'
}

// A reference is a file elsewhere (another host); it has no attachment block and shows as a plain tile.
const referenceBlocks = computed(() =>
  props.content.filter(
    (b): b is Extract<UserContentBlock, { type: 'reference' }> => b.type === 'reference',
  ),
)

function mediaName(block: MediaBlock, index: number): string {
  if (block.type === 'document') {
    return block.source.fileName
  }
  if (block.source.type === 'url') {
    const leaf = block.source.url.split('/').pop()
    if (leaf) {
      return decodeURIComponent(leaf)
    }
  }
  return `${block.type}-${index}`
}

const files = useMessageFiles()
const renderedMarkdown = computed(() => md.renderUser(userText.value, { files: files() }))

const textClass = computed(() => (props.pending ? 'text-fg-subtle' : 'text-fg-body'))

/** How many lines a long message shows; the last of them fades out. */
const VISIBLE_LINES = 5

const contentRef = ref<HTMLElement>()
const bodyRef = ref<HTMLElement>()
/**
 * Where a long message is cut, and the line box its fade spans; null while
 * it shows whole. The cut is five lines down, or lower, at the end of a line
 * of text across that mark, so it cuts no letter; anything else across it,
 * such as an image, a code block or a table, is cut there.
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
  const limit = VISIBLE_LINES * line
  const full = body.getBoundingClientRect().height
  if (full <= limit + 1) {
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
    // Text runs top to bottom: once it starts below the mark, the rest does too.
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
  // Five lines in paragraphs run past the mark by their gaps and show whole.
  clip.value = height < full - 1 ? { height, line } : null
}

// Wrapping follows the width, so a narrower or wider column changes the cut.
useResizeObserver([contentRef, bodyRef], measure)
</script>

<template>
  <div
    class="group/user relative z-10 flex flex-col items-end bg-surface px-[var(--agent-pad-x,2rem)] pb-2 pt-1.5"
    :class="forceStuck ? 'user-sticky' : ''"
  >
    <div class="relative max-w-[80%] rounded-xl bg-surface-raised p-2.5">
      <div
        v-if="actions.length > 0"
        class="user-actions absolute bottom-2.5 left-0 flex -translate-x-[calc(100%+6px)] items-center transition-opacity group-hover/user:opacity-100 focus-within:opacity-100"
        :class="actionsPinned ? 'opacity-100' : 'opacity-0'"
      >
        <Tooltip
          v-for="action in actions"
          :key="action.key"
          :content="action.hint"
          class="inline-flex"
        >
          <button
            type="button"
            :aria-label="action.hint"
            class="flex size-5 items-center justify-center rounded text-fg-faint transition-colors hover:bg-hover hover:text-fg-muted"
            @click.stop="action.emit()"
          >
            <component
              :is="action.icon"
              :size="13"
            />
          </button>
        </Tooltip>
      </div>
      <div
        v-if="
          fileTiles.length > 0 ||
          referenceBlocks.length > 0 ||
          (attachments && attachments.length > 0)
        "
        class="mb-2 flex flex-wrap gap-1.5"
      >
        <Tooltip
          v-for="item in attachments ?? []"
          :key="item.id"
          :content="attachmentCaption(item)"
        >
          <AttachmentTile
            :name="item.name"
            :src="item.kind === 'file' ? item.src : undefined"
          />
        </Tooltip>
        <template v-for="(tile, i) in fileTiles" :key="tile.key">
          <Tooltip
            v-if="tile.media"
            :content="tile.file ? `${tile.file.name} · ${tile.file.path}` : contentBlockCaption(tile.media)"
          >
            <ContentMedia
              :kind="tile.media.type"
              :name="tile.file?.name ?? mediaName(tile.media, i)"
              :source="tile.media.source"
            />
          </Tooltip>
          <Tooltip v-else-if="tile.file" :content="`${tile.file.name} · ${tile.file.path}`">
            <AttachmentTile :name="tile.file.name" :snippet="tile.file.snippet" />
          </Tooltip>
        </template>
        <Tooltip
          v-for="(block, i) in referenceBlocks"
          :key="`ref-${i}`"
          :content="contentBlockCaption(block)"
        >
          <AttachmentTile :name="decodeRemoteReference(block.reference).name" />
        </Tooltip>
      </div>
      <div
        ref="contentRef"
        class="overflow-hidden"
        :style="
          clip
            ? {
                height: `${clip.height}px`,
                maskImage: `linear-gradient(to bottom, black calc(100% - ${clip.line}px), transparent)`,
              }
            : undefined
        "
      >
        <div
          v-if="userText"
          ref="bodyRef"
          class="markdown-body select-text text-conversation"
          :class="textClass"
          v-html="renderedMarkdown"
          @click="openFileLink($event, files())"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Sits on the last text line: the bubble's bottom padding plus one line box, buttons centered in it. */
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

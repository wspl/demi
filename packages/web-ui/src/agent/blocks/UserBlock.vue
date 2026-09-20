<script setup lang="ts">
import { computed, ref } from 'vue'
import type { Component } from 'vue'
import { useClipboard, useResizeObserver } from '@vueuse/core'
import { ArrowUp, Check, ChevronsUp, Copy, Pencil, X } from '@lucide/vue'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import type { MessageEditContent } from '../message-editing'
import { composerCapsule, contentCapsule } from '../message-editor/capsules'
import MessageEditor from '../message-editor/MessageEditor.vue'
import type { ComposerAttachment } from '../message-input/attachments'
import { splitMessageContent } from '../message-input/message-content'

const props = defineProps<{
  content: readonly MessageEditContent[]
  /**
   * The files of a message the server has not confirmed yet, in the order of
   * the attachment marks in its text; its content is then text alone.
   */
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

/** The message as its editor shows it: its Markdown, and a capsule for each file in the order of their marks. */
const message = computed(() => {
  const { markdown, files } = splitMessageContent(props.content)
  return {
    markdown,
    capsules: props.attachments
      ? props.attachments.map(composerCapsule)
      : files.map((blocks, index) => contentCapsule(String(index), blocks)),
  }
})

/** What Copy puts on the clipboard: the Markdown, each file by its name. */
const copyText = computed(() => message.value.markdown
  .split(ATTACHMENT_MARK)
  .map((text, index) => `${text}${message.value.capsules[index]?.name ?? ''}`)
  .join(''))

// Sent messages get copy and edit; pending ones get the queue and steer controls instead.
const actions = computed<BubbleAction[]>(() => {
  const list: BubbleAction[] = []
  if (!props.pending) {
    list.push({
      key: 'copy',
      hint: copied.value ? 'Copied' : 'Copy',
      icon: copied.value ? Check : Copy,
      emit: () => void copy(copyText.value),
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

const textClass = computed(() => (props.pending ? 'text-fg-subtle' : 'text-fg-body'))

/** How many lines a long message shows; the last of them fades out. */
const VISIBLE_LINES = 5

const contentRef = ref<HTMLElement>()
const bodyRef = ref<HTMLElement>()
/**
 * Where a long message is cut, and the line box its fade spans; null while
 * it shows whole. The cut is five lines down, or lower, at the end of a line
 * of text across that mark, so it cuts no letter; anything else across it,
 * such as an image or a code block, is cut there.
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
  // Five lines around a code block run past the mark by its margins and show whole.
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
          ref="bodyRef"
          class="select-text text-conversation"
          :class="textClass"
        >
          <MessageEditor
            :markdown="message.markdown"
            :attachments="message.capsules"
          />
        </div>
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

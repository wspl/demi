<script setup lang="ts">
import { computed, ref } from 'vue'
import type { Component } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { X, ArrowUp, ChevronsUp } from '@lucide/vue'
import type { UserContentBlock } from '@demicodes/core'
import { md } from '@demicodes/web-ui/markdown/md'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { t } from '@demicodes/web-ui/infra/i18n'
import AttachmentTile from '../AttachmentTile.vue'

type ImageBlock = Extract<UserContentBlock, { type: 'image' }>

const props = defineProps<{
  content: UserContentBlock[]
  forceStuck?: boolean
  variant?: 'user' | 'steer'
  pending?: boolean
  deletable?: boolean
  sendable?: boolean
  interruptible?: boolean
  /** Keep the hover actions visible (a catalog specimen, not a hover). */
  actionsPinned?: boolean
}>()

const emit = defineEmits<{
  delete: []
  sendNow: []
  interrupt: []
}>()

interface BubbleAction {
  hint: string
  icon: Component
  emit: () => void
}

const actions = computed<BubbleAction[]>(() => {
  const list: BubbleAction[] = []
  if (props.deletable) {
    list.push({ hint: props.sendable ? t('agent.queue.remove') : t('agent.steer.discard'), icon: X, emit: () => emit('delete') })
  }
  if (props.sendable) list.push({ hint: t('agent.queue.sendNow'), icon: ArrowUp, emit: () => emit('sendNow') })
  if (props.interruptible) list.push({ hint: t('agent.steer.interrupt'), icon: ChevronsUp, emit: () => emit('interrupt') })
  return list
})

const userText = computed(() => {
  const firstText = props.content.find(
    (b): b is Extract<UserContentBlock, { type: 'text' }> => b.type === 'text',
  )
  return firstText?.text ?? ''
})

const imageBlocks = computed(() =>
  props.content.filter((b): b is ImageBlock => b.type === 'image'),
)

const documentBlocks = computed(() =>
  props.content.filter((b): b is Extract<UserContentBlock, { type: 'document' }> => b.type === 'document'),
)

function imageSrc(source: ImageBlock['source']): string {
  if (source.type === 'url') return source.url
  return URL.createObjectURL(new Blob([source.data as BlobPart], { type: source.mediaType }))
}

function imageName(source: ImageBlock['source'], index: number): string {
  if (source.type !== 'url') return `image-${index}`
  const leaf = source.url.split('/').pop()
  return leaf ? decodeURIComponent(leaf) : `image-${index}`
}

const renderedMarkdown = computed(() => md.renderUser(userText.value))

const textClass = computed(() => props.pending ? 'text-fg-subtle' : 'text-fg-body')

const contentRef = ref<HTMLElement>()
const isOverflowing = ref(false)

useResizeObserver(contentRef, () => {
  if (!contentRef.value) return
  isOverflowing.value = contentRef.value.scrollHeight > contentRef.value.clientHeight
})
</script>

<template>
  <div
    class="group/user relative z-10 flex flex-col items-end bg-surface px-[var(--agent-pad-x,2rem)] pb-2 pt-1.5"
    :class="forceStuck ? 'user-sticky' : ''"
  >
    <div class="relative max-w-[80%] rounded-xl bg-surface-raised p-2.5">
      <div
        v-if="actions.length > 0"
        class="absolute left-0 top-1/2 flex -translate-x-[calc(100%+6px)] -translate-y-1/2 items-center transition-opacity group-hover/user:opacity-100 focus-within:opacity-100"
        :class="actionsPinned ? 'opacity-100' : 'opacity-0'"
      >
        <Tooltip v-for="action in actions" :key="action.hint" :content="action.hint" class="inline-flex">
          <button
            type="button"
            :aria-label="action.hint"
            class="flex size-5 items-center justify-center rounded text-fg-faint transition-colors hover:bg-hover hover:text-fg-muted"
            @click.stop="action.emit()"
          >
            <component :is="action.icon" :size="13" />
          </button>
        </Tooltip>
      </div>
      <div v-if="imageBlocks.length > 0 || documentBlocks.length > 0" class="mb-2 flex flex-wrap gap-1.5">
        <AttachmentTile
          v-for="(block, i) in imageBlocks"
          :key="`img-${i}`"
          :name="imageName(block.source, i)"
          :src="imageSrc(block.source)"
        />
        <AttachmentTile
          v-for="(block, i) in documentBlocks"
          :key="`doc-${i}`"
          :name="block.source.fileName"
        />
      </div>
      <div
        ref="contentRef"
        class="max-h-48 overflow-hidden"
        :style="isOverflowing ? { maskImage: 'linear-gradient(to bottom, black calc(100% - 2rem), transparent)' } : undefined"
      >
        <div v-if="userText" class="markdown-body select-text text-conversation" :class="textClass" v-html="renderedMarkdown" />
      </div>
    </div>
  </div>
</template>

<style scoped>
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

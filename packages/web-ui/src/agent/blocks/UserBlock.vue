<script setup lang="ts">
import { computed, ref } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { CloseLine } from '@mingcute/vue/close'
import { FileLine } from '@mingcute/vue/file'
import type { UserContentBlock } from '@demi/core'
import { md } from '@demi/web-ui/markdown/md'

type ImageBlock = Extract<UserContentBlock, { type: 'image' }>

const props = defineProps<{
  content: UserContentBlock[]
  forceStuck?: boolean
  variant?: 'user' | 'steer'
  pending?: boolean
  deletable?: boolean
}>()

const emit = defineEmits<{
  delete: []
}>()

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

const renderedMarkdown = computed(() => md.renderUser(userText.value))

const contentRef = ref<HTMLElement>()
const isOverflowing = ref(false)

useResizeObserver(contentRef, () => {
  if (!contentRef.value) return
  isOverflowing.value = contentRef.value.scrollHeight > contentRef.value.clientHeight
})
</script>

<template>
  <div
    class="relative z-10 flex flex-col items-end bg-surface px-8 pb-2 pt-1.5"
    :class="forceStuck ? 'user-sticky' : ''"
  >
    <div
      class="group/user relative max-w-[80%] rounded-xl p-3"
      :class="props.variant === 'steer' ? 'bg-surface ring-1 ring-line-focus' : 'bg-surface-raised'"
    >
      <button
        v-if="deletable"
        type="button"
        aria-label="Delete pending steer"
        class="absolute left-0 top-1/2 flex size-5 -translate-x-[calc(100%+6px)] -translate-y-1/2 cursor-pointer items-center justify-center rounded text-fg-ghost transition-colors hover:bg-active hover:text-fg-muted group-hover/user:text-fg-subtle"
        @click.stop="emit('delete')"
      >
        <CloseLine :size="13" />
      </button>
      <div :class="pending ? 'opacity-50' : ''">
        <div v-if="imageBlocks.length > 0 || documentBlocks.length > 0" class="mb-2 flex flex-wrap gap-1.5">
          <img
            v-for="(block, i) in imageBlocks"
            :key="`img-${i}`"
            :src="imageSrc(block.source)"
            class="size-12 rounded-lg object-cover ring-1 ring-line"
          />
          <span
            v-for="(block, i) in documentBlocks"
            :key="`doc-${i}`"
            class="flex size-12 items-center justify-center rounded-lg bg-fg-ghost/60 text-fg-muted ring-1 ring-line"
            :title="block.source.fileName"
          >
            <FileLine :size="18" />
          </span>
        </div>
        <div
          ref="contentRef"
          class="max-h-48 overflow-hidden"
          :style="isOverflowing ? { maskImage: 'linear-gradient(to bottom, black calc(100% - 3rem), transparent)' } : undefined"
        >
          <div v-if="userText" class="markdown-body select-text text-sm leading-relaxed text-fg-body" v-html="renderedMarkdown" />
        </div>
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

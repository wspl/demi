<script setup lang="ts">
import type { QueuedMessage, UserContentBlock } from '@demi/core'

const props = defineProps<{
  queue: QueuedMessage[]
}>()

function messageText(message: QueuedMessage): string {
  return message.text || contentText(message.content)
}

function contentText(content: UserContentBlock[]): string {
  return content.map((block) => {
    if (block.type === 'text') return block.text
    if (block.type === 'reference') return block.reference
    if (block.type === 'document') return block.source.fileName
    return '[image]'
  }).join('\n')
}
</script>

<template>
  <div
    v-if="props.queue.length > 0"
    class="message-queue-bar mb-2 rounded-xl border border-line bg-surface-raised/95 px-3 py-2 backdrop-blur"
  >
    <div class="flex items-start gap-2">
      <div
        class="mt-0.5 flex h-5 min-w-5 shrink-0 items-center justify-center rounded-full bg-fg-ghost px-1.5 text-[11px] font-medium tabular-nums text-fg-muted"
      >
        {{ props.queue.length }}
      </div>
      <div class="min-w-0 flex-1 space-y-1">
        <div
          v-for="(message, index) in props.queue"
          :key="message.id"
          class="flex min-w-0 items-center gap-2 text-sm leading-5"
        >
          <span class="shrink-0 text-[11px] tabular-nums text-fg-faint">{{ index + 1 }}</span>
          <span class="min-w-0 truncate text-fg-body">{{ messageText(message) }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.message-queue-bar {
  box-shadow: var(--shadow-float);
}
</style>

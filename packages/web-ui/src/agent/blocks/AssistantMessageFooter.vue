<script setup lang="ts">
import { ref } from 'vue'
import { useClipboard } from '@vueuse/core'
import { Check, Copy, GitFork, LoaderCircle } from '@lucide/vue'
import type { MessageForkState } from '../message-fork'
import Tooltip from '../../ui/Tooltip.vue'
import RelativeTime from '../../ui/RelativeTime.vue'

const props = defineProps<{
  content: string
  createdAt: string
  fork?: () => Promise<void>
  forkState?: MessageForkState
}>()
const { copy, copied } = useClipboard({ copiedDuring: 1500 })
const copyError = ref(false)

async function copyMessage(): Promise<void> {
  copyError.value = false
  try {
    await copy(props.content)
  } catch {
    copyError.value = true
  }
}
</script>

<template>
  <div class="mt-2 flex flex-wrap items-center gap-1 text-fg-faint">
    <Tooltip :content="copied ? 'Copied' : 'Copy'">
      <button
        type="button"
        :aria-label="copied ? 'Copied' : 'Copy'"
        class="flex size-6 items-center justify-center rounded transition-colors hover:bg-hover hover:text-fg-muted"
        @click="copyMessage"
      >
        <Check v-if="copied" :size="13" />
        <Copy v-else :size="13" />
      </button>
    </Tooltip>
    <Tooltip :content="fork ? 'Fork' : 'Fork is available after this message completes.'">
      <button
        type="button"
        aria-label="Fork"
        :disabled="!fork || forkState?.phase === 'pending'"
        :aria-busy="forkState?.phase === 'pending'"
        class="flex size-6 items-center justify-center rounded transition-colors enabled:hover:bg-hover enabled:hover:text-fg-muted disabled:text-fg-ghost"
        @click="fork?.()"
      >
        <LoaderCircle v-if="forkState?.phase === 'pending'" :size="13" class="animate-spin" />
        <GitFork v-else :size="13" />
      </button>
    </Tooltip>
    <RelativeTime :timestamp="createdAt" class="ml-1 text-[11px] leading-6" />
    <span v-if="copyError" role="status" class="ml-1 text-[11px]">
      Could not copy. Try again.
    </span>
  </div>
</template>

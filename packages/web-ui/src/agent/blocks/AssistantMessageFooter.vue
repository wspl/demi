<script setup lang="ts">
import { ref } from 'vue'
import { useClipboard } from '@vueuse/core'
import { Check, Copy, GitFork, RefreshCw } from '@lucide/vue'
import type { MessageForkState } from '../message-fork'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import RelativeTime from '../../ui/RelativeTime.vue'

const props = defineProps<{
  content: string
  createdAt: string
  fork?: () => Promise<void>
  forkState?: MessageForkState
  /**
   * Asks for a new answer to the message before it, offered on the last
   * answer exactly when editing that message is (`message-editing.md`
   * § Regenerate and the Up Arrow key); absent elsewhere.
   */
  regenerate?: () => void
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
      <IconButton
        :icon="copied ? Check : Copy"
        size="sm"
        variant="ghost"
        :aria-label="copied ? 'Copied' : 'Copy'"
        @click="copyMessage"
      />
    </Tooltip>
    <Tooltip content="Fork" :disabled="!fork">
      <IconButton
        :icon="GitFork"
        size="sm"
        variant="ghost"
        aria-label="Fork"
        :disabled="!fork"
        disabled-reason="Fork is available after this message completes."
        :loading="forkState?.phase === 'pending'"
        @click="fork?.()"
      />
    </Tooltip>
    <Tooltip v-if="regenerate" content="Regenerate">
      <IconButton
        :icon="RefreshCw"
        size="sm"
        variant="ghost"
        aria-label="Regenerate"
        @click="regenerate()"
      />
    </Tooltip>
    <RelativeTime :timestamp="createdAt" class="ml-1 text-[11px] leading-6" />
    <span v-if="copyError" role="status" class="ml-1 text-[11px]">
      Could not copy. Try again.
    </span>
  </div>
</template>

<script setup lang="ts">
import { GitFork, RefreshCw } from '@lucide/vue'
import type { MessageForkState } from '../message-fork'
import CopyButton from '../../ui/CopyButton.vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import RelativeTime from '../../ui/RelativeTime.vue'

defineProps<{
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
</script>

<template>
  <div class="mt-2 flex flex-wrap items-center gap-1 text-fg-faint">
    <CopyButton :source="content" />
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
  </div>
</template>

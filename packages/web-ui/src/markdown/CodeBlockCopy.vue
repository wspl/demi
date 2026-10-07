<script setup lang="ts">
import { useClipboard } from '@vueuse/core'
import { Check, Copy } from '@lucide/vue'
import { showToast } from '../infra/toast'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'

/**
 * Copy at a message's code block's top-right corner: the code's exact text,
 * with the copied feedback a message's Copy gives. The block (`.code-block`,
 * which the message renderer wraps around each fenced block) places it and
 * shows it on hover and focus, always on a touch screen.
 */
const props = defineProps<{
  /** The rendered block, whose `code` element holds the code as written. */
  block: HTMLElement
}>()
const { copy, copied } = useClipboard({ copiedDuring: 1500 })

async function copyCode(): Promise<void> {
  try {
    await copy(props.block.querySelector('code')?.textContent ?? '')
  } catch (error) {
    showToast({
      title: 'Could Not Copy',
      message: error instanceof Error ? error.message : String(error),
      tone: 'danger',
    })
  }
}
</script>

<template>
  <span class="code-block-copy">
    <Tooltip :content="copied ? 'Copied' : 'Copy'">
      <IconButton
        :icon="copied ? Check : Copy"
        size="sm"
        variant="ghost"
        :aria-label="copied ? 'Copied' : 'Copy'"
        @click="copyCode"
      />
    </Tooltip>
  </span>
</template>

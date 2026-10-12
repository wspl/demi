<script setup lang="ts">
import CopyButton from '../ui/CopyButton.vue'

/**
 * Copy at a message's code block's top-right corner: the code's exact text,
 * with the copied feedback a message's Copy gives. The block (`.code-block`,
 * which the message renderer wraps around each fenced block) places it and
 * shows it on hover and focus (`base.css`).
 */
const props = defineProps<{
  /** The rendered block, whose `code` element holds the code as written. */
  block: HTMLElement
}>()
</script>

<template>
  <span class="code-block-copy">
    <CopyButton :source="() => props.block.querySelector('code')?.textContent ?? ''" />
  </span>
</template>

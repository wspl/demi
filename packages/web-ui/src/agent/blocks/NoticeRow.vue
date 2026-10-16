<script setup lang="ts">
import FunctionalBlock from './FunctionalBlock.vue'
import type { ToolPicture } from '../tool-media'

/**
 * A notice row in the transcript, for a command's end and an agent's
 * receipt alike (`shell-tool.md` § Command reports, `subagents.md` § Product
 * rendering): its icon, then what its face says (`CommandReportFace`,
 * `AgentReceiptFace`), and the pictures a report carries after it. It does
 * not open: no chevron, no body, no toggle; its title, the one control,
 * leads where the detail is.
 */
defineProps<{
  /** The images and videos a command's end report carries, small in the row's line. */
  pictures?: readonly ToolPicture[]
}>()
</script>

<template>
  <FunctionalBlock :expandable="false" :pictures="pictures">
    <template #icon>
      <slot name="icon" />
    </template>
    <slot />
  </FunctionalBlock>
</template>

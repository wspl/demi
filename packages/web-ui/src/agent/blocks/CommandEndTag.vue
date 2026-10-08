<script setup lang="ts">
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import type { CommandEndMark } from '../command-end'

/**
 * A shell row's mark of a command that went wrong, after its title: red
 * Failed with the exit code in its tooltip, or grey Stopped. It never
 * shrinks, so a long title gives up its end to it.
 */
defineProps<{ mark: CommandEndMark }>()
</script>

<template>
  <Tooltip
    v-if="mark.kind === 'failed'"
    :content="`Exit code ${mark.exitCode}`"
    class="flex shrink-0 items-center"
  >
    <Tag tone="danger">Failed</Tag>
  </Tooltip>
  <Tag v-else class="shrink-0">Stopped</Tag>
</template>

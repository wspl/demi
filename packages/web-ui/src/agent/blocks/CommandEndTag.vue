<script setup lang="ts">
import { upperFirst } from '@demicodes/utils'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import type { CommandEndMark } from '../command-end'

/**
 * A row's mark of a command that went wrong, after its title: red Failed
 * with the exit code in its tooltip, grey Stopped, or grey Lost with why in
 * its tooltip. It never shrinks, so a long title gives up its end to it.
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
  <Tooltip
    v-else-if="mark.kind === 'lost'"
    :content="upperFirst(mark.reason)"
    class="flex shrink-0 items-center"
  >
    <Tag>Lost</Tag>
  </Tooltip>
  <Tag v-else class="shrink-0">Stopped</Tag>
</template>

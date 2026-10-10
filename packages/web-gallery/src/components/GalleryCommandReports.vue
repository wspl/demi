<script setup lang="ts">
import type { CommandReport } from '@demicodes/protocol'
import CommandReportBlock from '@demicodes/web-ui/agent/blocks/CommandReportBlock.vue'
import { provideCommandOpener } from '@demicodes/web-ui/agent/command-reports'
import { productWould } from '../product-would'

/**
 * A `wakeup` block's report rows. A click on a row says what the product
 * would do, open the command's terminal tab, since the gallery has no
 * terminal panel beside the specimen; a command no terminal tab shows,
 * `untracked`, is no control.
 */
const props = defineProps<{
  reports: CommandReport[]
  untracked?: string[]
}>()

provideCommandOpener((commandId) => props.untracked?.includes(commandId)
  ? undefined
  : () => productWould(`The Terminal Panel Opens on Command ${commandId}`))
</script>

<template>
  <CommandReportBlock :reports="reports" class="[--agent-pad-x:0px]" />
</template>

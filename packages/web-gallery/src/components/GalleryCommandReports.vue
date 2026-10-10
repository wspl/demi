<script setup lang="ts">
import type { CommandReport } from '@demicodes/protocol'
import CommandReportBlock from '@demicodes/web-ui/agent/blocks/CommandReportBlock.vue'
import { provideCommandRevealer } from '@demicodes/web-ui/agent/command-reports'
import { productWould } from '../product-would'

/**
 * A `wakeup` block's report rows, shown without the transcript they lie in:
 * a click on a title says what the product would do, bring the call that
 * started the command into view.
 */
defineProps<{ reports: CommandReport[] }>()

provideCommandRevealer((commandId) => () => productWould(`The Transcript Scrolls to the Call That Started Command ${commandId}`))
</script>

<template>
  <CommandReportBlock :reports="reports" class="[--agent-pad-x:0px]" />
</template>

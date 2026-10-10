<script setup lang="ts">
import { computed } from 'vue'
import type { CommandReport } from '@demicodes/protocol'
import { SquareTerminal } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from './FunctionalBlock.vue'
import CommandEndTag from './CommandEndTag.vue'
import ToolMedia from './ToolMedia.vue'
import { reportMark, reportSentence, useCommandOpener } from '../command-reports'
import { toolMedia } from '../tool-media'

/**
 * The reports of a `wakeup` block, one row each where the block lies, as an
 * agent message shows as its receipt row (`runtime.md` § Command reports): a
 * reply the agent writes after a report never appears without its cause. A
 * click on a row opens its command's terminal tab. The media an end report
 * carries show under its row, as a call's do (`file-previews.md` § Media a
 * tool returned).
 */
const props = defineProps<{ reports: readonly CommandReport[] }>()

const open = useCommandOpener()
const rows = computed(() => props.reports.map((report, index) => ({
  key: `${report.commandId}:${index}`,
  sentence: reportSentence(report),
  mark: reportMark(report),
  action: open(report.commandId),
  media: toolMedia(report.media ?? [], report.title),
})))
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <template
      v-for="row in rows"
      :key="row.key"
    >
      <FunctionalBlock :action="row.action">
        <template #icon>
          <SquareTerminal :size="ICON_PX.in28" />
        </template>
        <span class="min-w-0 truncate">{{ row.sentence }}</span>
        <CommandEndTag v-if="row.mark" :mark="row.mark" />
      </FunctionalBlock>
      <ToolMedia :media="row.media" />
    </template>
  </div>
</template>

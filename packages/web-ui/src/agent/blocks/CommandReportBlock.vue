<script setup lang="ts">
import { computed } from 'vue'
import type { CommandReport } from '@demicodes/protocol'
import { Bell } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from './FunctionalBlock.vue'
import CommandEndTag from './CommandEndTag.vue'
import ToolMedia from './ToolMedia.vue'
import { reportMark, reportNotice, reportTitle, useCommandRevealer } from '../command-reports'
import { toolMedia } from '../tool-media'
import { provideBlockScope } from '../whole-blocks'

/**
 * The ends a `wakeup` block reports, one notice row each where the block
 * lies (`runtime.md` § Command reports): a reply the agent writes after a
 * command's end never appears without its cause. A report of progress shows
 * nothing; the agent's answer to it says what it learned. The notice names
 * the call that started the command, which a click brings into view. The
 * media an end report carries show under its row, as a call's do
 * (`file-previews.md` § Media a tool returned).
 */
const props = defineProps<{ reports: readonly CommandReport[] }>()

// What a notice shows, the light form keeps: its rows never open to read the block whole.
provideBlockScope(() => undefined)
const reveal = useCommandRevealer()
const rows = computed(() => props.reports.flatMap((report, index) => {
  const notice = reportNotice(report)
  return notice === null
    ? []
    : [{
        key: `${report.commandId}:${index}`,
        notice,
        title: reportTitle(report),
        mark: reportMark(report),
        go: reveal(report.commandId),
        media: toolMedia(report.media ?? [], report.title),
      }]
}))
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <template
      v-for="row in rows"
      :key="row.key"
    >
      <FunctionalBlock>
        <template #icon>
          <Bell :size="ICON_PX.in28" />
        </template>
        <span class="shrink-0">{{ row.notice }}:</span>
        <!-- The call's title is a link to the call, which a long title keeps while it truncates. -->
        <button
          v-if="row.go"
          type="button"
          class="min-w-0 cursor-pointer truncate text-fg-muted underline decoration-dotted decoration-fg-faint underline-offset-3 transition-colors duration-200 ease-out hover:text-fg-body hover:decoration-current"
          @click="row.go"
        >{{ row.title }}</button>
        <span v-else class="min-w-0 truncate text-fg-muted">{{ row.title }}</span>
        <CommandEndTag v-if="row.mark" :mark="row.mark" />
      </FunctionalBlock>
      <ToolMedia :media="row.media" />
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { CommandReport } from '@demicodes/protocol'
import { Bell } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import NoticeRow from './NoticeRow.vue'
import CommandEndTag from './CommandEndTag.vue'
import ToolMedia from './ToolMedia.vue'
import { reportMark, reportNotice, reportTitle, useCommandRevealer } from '../command-reports'
import { reportMedia } from '../tool-media'
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
const props = defineProps<{
  reports: readonly CommandReport[]
  /** The notice is a lone step of the running turn, its activity: its newest row shimmers. */
  working?: boolean
}>()

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
        media: reportMedia(report),
      }]
}))
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <template
      v-for="(row, index) in rows"
      :key="row.key"
    >
      <NoticeRow :notice="row.notice" :title="row.title" :go="row.go" :loading="working && index === rows.length - 1">
        <template #icon>
          <Bell :size="ICON_PX.in28" />
        </template>
        <template v-if="row.mark" #tag>
          <CommandEndTag :mark="row.mark" />
        </template>
      </NoticeRow>
      <ToolMedia :media="row.media" />
    </template>
  </div>
</template>

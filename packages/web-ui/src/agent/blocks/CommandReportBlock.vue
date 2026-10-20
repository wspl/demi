<script setup lang="ts">
import { computed } from 'vue'
import type { CommandReport } from '@demicodes/protocol'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import NoticeRow from './NoticeRow.vue'
import CommandReportFace from './CommandReportFace.vue'
import { NOTICE_ICON } from './tool-row-icon'
import GoneMediaLines from './GoneMediaLines.vue'
import { reportShown } from '../command-reports'
import { reportMedia, splitMedia } from '../tool-media'
import { provideBlockScope } from '../whole-blocks'

/**
 * The ends a `wakeup` block reports, one notice row each where the block
 * lies (`crates/agent-session/src/reports.rs`): a reply the agent writes after a
 * command's end never appears without its cause. A report of progress shows
 * nothing, and a block of progress reports alone has no row: the agent's
 * answer says what it learned, and a row for every interval would outnumber
 * the answers it caused. The notice names
 * the call that started the command, which a click brings into view. The
 * pictures an end report carries stand in its row's line, as a call's do
 * (`file-previews.md` § Media a tool returned).
 */
const props = defineProps<{
  reports: readonly CommandReport[]
}>()

// What a notice shows, the light form keeps: its rows never open to read the block whole.
provideBlockScope(() => undefined)
const rows = computed(() => props.reports.flatMap((report, index) => reportShown(report)
  ? [{
      key: `${report.commandId}:${index}`,
      report,
      media: splitMedia(reportMedia(report)),
    }]
  : []))
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <template
      v-for="(row, index) in rows"
      :key="row.key"
    >
      <NoticeRow :pictures="row.media.pictures">
        <template #icon>
          <component :is="NOTICE_ICON" :size="ICON_PX.in28" />
        </template>
        <CommandReportFace :report="row.report" />
      </NoticeRow>
      <GoneMediaLines :lines="row.media.gone" />
    </template>
  </div>
</template>

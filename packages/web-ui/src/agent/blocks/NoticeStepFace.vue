<script setup lang="ts">
import { computed } from 'vue'
import NoticeLabel from './NoticeLabel.vue'
import CommandEndTag from './CommandEndTag.vue'
import ReceiptTag from './ReceiptTag.vue'
import { receiptRow } from '../agent-receipts'
import { reportMark, reportNotice, reportShown, reportTitle } from '../command-reports'
import type { NoticeStep } from '../work-groups'

/**
 * A notice or a receipt as the face of a row that stands for it, the run's
 * row while it is the newest step or the tail row it rolls through: the
 * words and the tag of its own notice row, the newest one of a `wakeup`
 * that reports several ends. The face is the row's toggle, so its title is
 * plain text; the step's own row, open, links it.
 */
const props = defineProps<{ step: NoticeStep }>()

const face = computed(() => {
  const step = props.step
  if (step.type === 'agent_message') {
    const row = receiptRow(step.message)
    return { notice: row.notice, title: row.title, subject: row.subject, tag: row.tag, mark: null }
  }
  const report = step.reports.findLast(reportShown)!
  return { notice: reportNotice(report) ?? '', title: reportTitle(report), subject: false, tag: null, mark: reportMark(report) }
})
</script>

<template>
  <NoticeLabel :notice="face.notice" :title="face.title" :subject="face.subject" />
  <CommandEndTag v-if="face.mark" :mark="face.mark" />
  <ReceiptTag v-else-if="face.tag" :tag="face.tag" />
</template>

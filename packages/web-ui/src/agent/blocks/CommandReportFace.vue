<script setup lang="ts">
import { computed } from 'vue'
import type { CommandReport } from '@demicodes/protocol'
import NoticeLabel from './NoticeLabel.vue'
import CommandEndTag from './CommandEndTag.vue'
import { reportMark, reportNotice, reportTitle, useCommandRevealer } from '../command-reports'

/**
 * What a command's end says on its notice row
 * (`crates/agent-session/src/reports.rs`), wherever the row stands, the folded
 * face of a run included: the words, the call's title, which a click brings
 * into view, and the tag of what went wrong.
 */
const props = defineProps<{ report: CommandReport }>()

const reveal = useCommandRevealer()
const go = computed(() => reveal(props.report.commandId))

const notice = computed(() => reportNotice(props.report) ?? '')
const mark = computed(() => reportMark(props.report))
</script>

<template>
  <NoticeLabel :notice="notice" :title="reportTitle(report)" :go="go" />
  <CommandEndTag v-if="mark" :mark="mark" />
</template>

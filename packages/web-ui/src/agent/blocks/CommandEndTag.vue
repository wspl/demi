<script setup lang="ts">
import { computed } from 'vue'
import { upperFirst } from '@demicodes/utils'
import type { CommandEndMark } from '../command-end'
import RunEndTag from './RunEndTag.vue'
import { t } from '../locales/text'

/**
 * A row's mark of a command that went wrong, after its title:
 *
 * - red Failed, with the exit code in its tooltip
 * - grey Stopping… while a stop waits for the command's runner to end it
 * - grey Waiting for MacBook Pro while its start waits for its offline Host
 * - grey Stopped
 * - grey Lost, with why in its tooltip
 */
const props = defineProps<{ mark: CommandEndMark }>()

const hint = computed(() => {
  if (props.mark.kind === 'failed')
    return t('transcript.runEnd.exitCode', { code: props.mark.exitCode })
  if (props.mark.kind === 'lost')
    return upperFirst(props.mark.reason)
  return undefined
})
</script>

<template>
  <RunEndTag :kind="mark.kind" :hint="hint" :host="mark.kind === 'waiting' ? mark.host : undefined" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { ProviderErrorDiagnostics } from '@demicodes/core'
import { t } from '@demicodes/web-ui/infra/i18n'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import { errorFacts, errorReportText, errorSummary } from '../error-detail'

/**
 * The transcript record of a turn that failed: one sentence from the
 * normalized code, the upstream message, the diagnostics and Copy. The record
 * at the tail of an idle conversation carries Retry, so recovery starts where
 * the failure is read.
 */
const props = defineProps<{
  /** The upstream error as the provider reported it. */
  message: string
  code?: string | null
  diagnostics?: ProviderErrorDiagnostics
  /** Offered only on the record that ended the conversation. */
  retry?: () => void
}>()

const summary = computed(() => errorSummary(props.code))
const facts = computed(() => errorFacts(props.code, props.diagnostics))
const reportText = computed(() =>
  errorReportText(props.message, props.code, props.diagnostics),
)
</script>

<template>
  <ErrorNotice
    :label="summary"
    :detail="message"
    :facts="facts"
    :copy-text="reportText"
    :action="retry ? t('agent.session.retry') : undefined"
    @action="retry?.()"
  />
</template>

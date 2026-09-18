<script setup lang="ts">
import { computed } from 'vue'
import type { ProviderErrorDiagnostics } from '@demicodes/core'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import { errorFacts, errorReportText, errorSummary } from '../error-detail'

/**
 * The transcript record of a turn that failed: a neutral sentence, the
 * message in its source's own words, the diagnostics and Copy. It says what
 * happened and offers no action: recovery sits in the dock above the composer
 * (`product.md` § Recovering an unfinished turn).
 */
const props = defineProps<{
  /** The upstream error as the provider reported it. */
  message: string
  code?: string | null
  diagnostics?: ProviderErrorDiagnostics
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
  />
</template>

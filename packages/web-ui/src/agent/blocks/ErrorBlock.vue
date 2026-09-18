<script setup lang="ts">
import { computed } from 'vue'
import type { ProviderErrorDiagnostics } from '@demicodes/core'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import { errorFacts, errorPresentation, errorReportText } from '../error-detail'

/**
 * The transcript record of a turn that failed: what its source said on the
 * first line, the full text below only when there is more, the diagnostics
 * and Copy. It says what
 * happened and offers no action: recovery sits in the dock above the composer
 * (`product.md` § Recovering an unfinished turn).
 */
const props = defineProps<{
  /** The upstream error as the provider reported it. */
  message: string
  code?: string | null
  diagnostics?: ProviderErrorDiagnostics
}>()

const presentation = computed(() => errorPresentation(props.message))
const facts = computed(() => errorFacts(props.code, props.diagnostics))
const reportText = computed(() =>
  errorReportText(props.message, props.code, props.diagnostics),
)
</script>

<template>
  <ErrorNotice
    :label="presentation.label"
    :detail="presentation.detail"
    :facts="facts"
    :copy-text="reportText"
  />
</template>

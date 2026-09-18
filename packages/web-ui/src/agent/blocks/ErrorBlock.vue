<script setup lang="ts">
import { computed } from 'vue'
import type { ProviderErrorDiagnostics } from '@demicodes/core'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import { errorFacts, errorPresentation, errorReportText, prettyUpstream } from '../error-detail'

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
  /** When the failure was recorded; a relative vendor wait counts from here. */
  createdAt?: string
}>()

const presentation = computed(() => errorPresentation(props.message))
const facts = computed(() => errorFacts(props.code, props.diagnostics, props.createdAt))
const raw = computed(() =>
  props.diagnostics?.upstream ? prettyUpstream(props.diagnostics.upstream) : null,
)
const reportText = computed(() =>
  errorReportText(props.message, props.code, props.diagnostics),
)
</script>

<template>
  <ErrorNotice
    :label="presentation.label"
    :detail="presentation.detail"
    :facts="facts"
    :raw="raw"
    :copy-text="reportText"
  />
</template>

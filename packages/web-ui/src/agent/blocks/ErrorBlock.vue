<script setup lang="ts">
import { computed } from 'vue'
import type { ProviderErrorDiagnostics } from '@demicodes/core'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import { errorFacts, errorPresentation, errorReportText, prettyUpstream } from '../error-detail'

/**
 * The transcript record of a turn that failed: what its source said on the
 * first line, the full text below only when there is more, when the provider
 * says it works again, the provider's response as it arrived, and Copy. It
 * says what happened and offers no action: recovery sits in the dock above
 * the composer (`product.md` § Recovering an unfinished turn).
 */
const props = defineProps<{
  /** The upstream error as the provider reported it. */
  message: string
  code?: string | null
  diagnostics?: ProviderErrorDiagnostics
  /** When the provider says the request can succeed again, as the backend read it. */
  retryAt?: string | null
}>()

const presentation = computed(() => errorPresentation(props.message))
const facts = computed(() => errorFacts(props.retryAt ?? null))
// The provider's response as it arrived, whole.
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

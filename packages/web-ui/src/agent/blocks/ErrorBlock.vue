<script setup lang="ts">
import { computed } from 'vue'
import type { ProviderErrorDiagnostics } from '@demicodes/protocol'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import Checkbox from '../../ui/Checkbox.vue'
import { useClock } from '../../composables/useClock'
import { useTranscript } from '../edit-selection'
import { errorReportText, failureHeadline, failureSource } from '../error-detail'
import { resumeAtLabel, useResumeAtRetry } from '../resume-at-retry'

/**
 * The transcript record of a turn that failed: a headline the page words
 * from the failure's code and when the provider says it works again, the
 * provider's response as it arrived behind a disclosure, and Copy. It
 * says what happened and offers no action: recovery sits in the dock above
 * the composer (`product.md` § Recovering an unfinished turn). A failure of
 * the conversation's own agent that names when it works again carries the
 * one switch that belongs to it until that time: Resume at that time, the
 * user's preference for every such turn.
 */
const props = defineProps<{
  /** The upstream error as the provider reported it. */
  message: string
  code?: string | null
  diagnostics?: ProviderErrorDiagnostics
  /** When the provider says the request can succeed again, as the backend read it. */
  retryAt?: string | null
}>()

const source = computed(() => failureSource(props.message, props.code, props.diagnostics))
const reportText = computed(() =>
  errorReportText(props.message, props.code, props.diagnostics),
)

const resumeAtRetry = useResumeAtRetry()
const transcript = useTranscript()
const now = useClock()
const label = computed(() => failureHeadline(props.code, props.retryAt ?? null, new Date(now.value)))
const ownAgent = transcript === null || transcript.node === null
/** The preference this record offers, until its time has passed. */
const resume = computed(() => {
  const preference = resumeAtRetry()
  const label = resumeAtLabel(props.retryAt ?? null, ownAgent, preference !== undefined, now.value)
  return preference && label ? { preference, label } : null
})
</script>

<template>
  <ErrorNotice
    :label="label"
    :detail="source.detail"
    :raw="source.raw"
    :copy-text="reportText"
  >
    <Checkbox
      v-if="resume"
      class="mt-0.5"
      :label="resume.label"
      :model-value="resume.preference.on"
      @update:model-value="resume.preference.set($event)"
    />
  </ErrorNotice>
</template>

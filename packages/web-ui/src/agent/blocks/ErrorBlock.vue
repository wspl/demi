<script setup lang="ts">
import { computed } from 'vue'
import type { ProviderErrorDiagnostics } from '@demicodes/protocol'
import ErrorNotice from '@demicodes/web-ui/ui/ErrorNotice.vue'
import Checkbox from '../../ui/Checkbox.vue'
import { useClock } from '../../composables/useClock'
import { useTranscript } from '../edit-selection'
import { useCredentialKinds } from '../credential-kinds'
import { errorReportText, failureSource, recordHeadline } from '../error-detail'
import { resumeAtLabel, useResumeAtRetry } from '../resume-at-retry'

/**
 * The transcript record of a turn that failed: a headline the page words from
 * the failure's code and when the provider says it works again, the provider's
 * response as it arrived behind a disclosure, and Copy.
 *
 * It says what happened and offers no action: recovery sits in the dock above
 * the composer. A failure of the conversation's own agent that names when it
 * works again carries the one switch that belongs to it until that time: Resume
 * at that time, the user's preference for every such turn.
 */
const props = defineProps<{
  /** The upstream error as the provider reported it. */
  message: string
  code?: string | null
  diagnostics?: ProviderErrorDiagnostics
  /**
   * The entry the block's model selection names, whose credential the headline
   * names.
   */
  providerId?: string
  /**
   * When the provider says the request can succeed again, as the backend read
   * it.
   */
  retryAt?: string | null
  /**
   * The account the vendor signed out, while it still is, which the record
   * names instead of the code's words.
   */
  signedOut?: { provider: string; account: string } | null
  /**
   * The device's port the request named, while it is still not exposed, which
   * the record names instead of the code's words.
   */
  notExposed?: { device: string; port: number } | null
  /**
   * The device the request named while it was offline, which the record names.
   */
  deviceOffline?: { device: string } | null
}>()

const source = computed(() => failureSource(props.message, props.code, props.diagnostics))
const reportText = computed(() =>
  errorReportText(props.message, props.code, props.diagnostics),
)

const resumeAtRetry = useResumeAtRetry()
const transcript = useTranscript()
const now = useClock()
const credentialKinds = useCredentialKinds()
const label = computed(() =>
  recordHeadline(
    props.code,
    {
      retryAt: props.retryAt ?? null,
      signedOut: props.signedOut ?? null,
      notExposed: props.notExposed ?? null,
      deviceOffline: props.deviceOffline ?? null,
      credential: props.providerId === undefined ? null : credentialKinds()?.(props.providerId) ?? null,
    },
    new Date(now.value),
  ),
)
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
    :raw-headers="source.headers"
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

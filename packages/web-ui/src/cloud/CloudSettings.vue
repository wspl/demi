<script setup lang="ts">
import { computed, ref } from 'vue'
import type { CloudState } from './types'
import { formatBytes } from '../files/format'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import SettingsGroup from '../settings/SettingsGroup.vue'
import SettingsRow from '../settings/SettingsRow.vue'
import CloudResetDialog from './CloudResetDialog.vue'

const props = defineProps<{
  cloud: CloudState
  resetPending?: boolean
  resetError?: string | null
  overlayStore: OverlayStore
}>()
const emit = defineEmits<{
  reset: [operationId: string]
}>()
const open = ref(false)
const submitted = ref(false)
const operationId = ref('')
/**
 * Whether the Cloud's latest reset is the one this dialog asked for. Until
 * the status names it, the phase and failure there belong to an earlier
 * reset: after one that ended ready, a new confirmation would otherwise read
 * "Cloud is ready." while its own request is still pending.
 */
const own = computed(() => submitted.value && props.cloud.operationId === operationId.value)
const phase = computed(() => (own.value ? props.cloud.phase : null))
const error = computed(() => props.resetError || (own.value ? props.cloud.error : null))
const busy = computed(
  () =>
    props.resetPending ||
    props.cloud.state === 'resetting' ||
    (submitted.value &&
      !props.resetError &&
      phase.value !== 'ready' &&
      phase.value !== 'failed'),
)
/**
 * Each filesystem's current size against the most it may grow to; before the
 * Cloud's first start there is no size yet, only the limit.
 */
const storage = computed(() => {
  const { volumes, limits } = props.cloud
  const size = (current: number | undefined, limit: number) =>
    current === undefined ? `up to ${formatBytes(limit)}` : `${formatBytes(current)} of ${formatBytes(limit)}`
  return `System: ${size(volumes?.systemBytes, limits.systemBytes)} · Home: ${size(volumes?.homeBytes, limits.homeBytes)}`
})
function begin() {
  submitted.value = false
  operationId.value = crypto.randomUUID()
  open.value = true
}
function reset() {
  if (busy.value) {
    return
  }
  submitted.value = true
  emit('reset', operationId.value)
}
</script>

<template>
  <SettingsGroup title="Cloud">
    <SettingsRow
      label="Your Cloud environment"
      description="All your Cloud projects share this environment. Starts automatically when needed."
    >
      <Button
        size="sm"
        :disabled="cloud.state === 'resetting'"
        @click="begin"
        >Reset environment</Button
      >
    </SettingsRow>
    <SettingsRow label="Storage" :description="storage" />
    <CloudResetDialog
      :is-open="open"
      :overlay-store="overlayStore"
      :phase="phase"
      :submitted="submitted"
      :error="error"
      :busy="busy"
      @close="open = false"
      @reset="reset"
    />
  </SettingsGroup>
</template>

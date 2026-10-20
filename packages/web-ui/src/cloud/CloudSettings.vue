<script setup lang="ts">
import { t as devicesText } from '../devices/locales/text'
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import { computed, ref } from 'vue'
import type { CloudState } from './types'
import { formatBytes } from '../files/format'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import SettingsGroup from '../settings/SettingsGroup.vue'
import SettingsRow from '../settings/SettingsRow.vue'
import CloudResetDialog from './CloudResetDialog.vue'

// Settings shows the Cloud's storage and Reset Environment, not its lifecycle
// state: a stopped Cloud is not unavailable, since the next operation that needs
// it wakes it, and a label such as "Sleeping" would suggest otherwise.

const props = defineProps<{
  cloud: CloudState
  resetError?: ShownError | null
  overlayStore: OverlayStore
}>()
const emit = defineEmits<{
  reset: [operationId: string]
  /** Opens the Cloud's own page. */
  open: []
}>()
const open = ref(false)
const submitted = ref(false)
const operationId = ref('')
/**
 * Whether the Cloud's latest reset is the one this dialog asked for.
 *
 * Until the status names it, the phase and failure there belong to an earlier reset.
 * After one that ended ready, a new confirmation would otherwise read "Cloud is
 * ready." while its own request is still pending.
 */
const own = computed(() => submitted.value && props.cloud.operationId === operationId.value)
const phase = computed(() => (own.value ? props.cloud.phase : null))
const error = computed<ShownError | null>(() => {
  if (props.resetError) {
    return props.resetError
  }
  // The Cloud's own record of the failure, which no request answered.
  return own.value && props.cloud.error ? { message: props.cloud.error, requestId: null } : null
})
// From the press on: the dialog's own request, then the Cloud's phases.
const busy = computed(
  () =>
    props.cloud.state === 'resetting' ||
    (submitted.value &&
      !props.resetError &&
      phase.value !== 'ready' &&
      phase.value !== 'failed'),
)
/** Each filesystem's capacity; none before the Cloud's first start made them. */
const storage = computed(() => {
  const { volumes } = props.cloud
  return volumes
    ? t('storage.line', { system: formatBytes(volumes.systemBytes), home: formatBytes(volumes.homeBytes) })
    : null
})
/** The Cloud's one line, as a device's row has: its system and how this page reaches it. */
const line = computed(() => {
  const system = props.cloud.report.os?.name
  return system ? t('environment.connected', { system }) : devicesText('connected.relay')
})
function begin() {
  submitted.value = false
  open.value = true
}
/** Every request is a reset of its own, a retry too, so its status is never an earlier one's. */
function reset() {
  if (busy.value) {
    return
  }
  submitted.value = true
  operationId.value = crypto.randomUUID()
  emit('reset', operationId.value)
}
</script>

<template>
  <SettingsGroup
    :title="t('settings.title')"
    :description="t('settings.description')"
  >
    <SettingsRow
      :label="t('environment.title')"
      :description="line"
      :navigable="cloud.deviceId !== null"
      isolate-controls
      @click="emit('open')"
    >
      <Button
        size="sm"
        :disabled="cloud.state === 'resetting'"
        @click="begin"
        >{{ t('environment.reset') }}</Button
      >
    </SettingsRow>
    <SettingsRow
      v-if="cloud.newerImage"
      :label="t('environment.newerImage')"
      :description="t('environment.newerImageDescription')"
    />
    <SettingsRow
      v-if="storage"
      :label="t('storage.title')"
      :description="storage"
    />
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

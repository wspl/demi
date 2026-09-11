<script setup lang="ts">
import { computed, ref } from 'vue'
import type { CloudState } from './types'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import SettingsGroup from '../settings/SettingsGroup.vue'
import SettingsRow from '../settings/SettingsRow.vue'
import CloudResetDialog from './CloudResetDialog.vue'
import { resetPhaseLabels } from './reset-phases'

const props = defineProps<{
  cloud: CloudState
  resetPending?: boolean
  resetError?: string | null
  overlayStore: OverlayStore
}>()
const emit = defineEmits<{ reset: [operationId: string] }>()
const open = ref(false)
const submitted = ref(false)
const operationId = ref('')
const busy = computed(
  () =>
    props.resetPending ||
    props.cloud.state === 'resetting' ||
    (submitted.value &&
      !props.resetError &&
      props.cloud.phase !== 'ready' &&
      props.cloud.phase !== 'failed'),
)
const label = computed(() =>
  props.cloud.state === 'resetting' && props.cloud.phase
    ? resetPhaseLabels[props.cloud.phase]
    : {
        unallocated: 'Starts when you need it',
        off: 'Sleeping',
        booting: 'Starting…',
        running: 'Running',
        saving: 'Saving…',
        resetting: 'Resetting…',
        unavailable: 'Unavailable',
      }[props.cloud.state],
)
function begin() {
  submitted.value = false
  operationId.value = crypto.randomUUID()
  open.value = true
}
function reset() {
  if (busy.value || props.cloud.state === 'unavailable') {
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
      :description="`${label} · All your Cloud projects share this machine.`"
    >
      <Button
        size="sm"
        :disabled="cloud.state === 'unavailable' || cloud.state === 'resetting'"
        @click="begin"
        >Reset environment</Button
      >
    </SettingsRow>
    <SettingsRow
      label="Storage limits"
      :description="`System: ${Math.round(cloud.systemBytes / 1024 ** 3)} GiB · Home: ${Math.round(cloud.homeBytes / 1024 ** 3)} GiB`"
    />
    <CloudResetDialog
      :is-open="open"
      :overlay-store="overlayStore"
      :phase="cloud.phase"
      :submitted="submitted"
      :error="resetError || cloud.error || null"
      :busy="busy"
      @close="open = false"
      @reset="reset"
    />
  </SettingsGroup>
</template>

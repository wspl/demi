<script setup lang="ts">
import { computed, ref } from 'vue'
import type { CloudState } from './types'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import InlineError from '../ui/InlineError.vue'
import SettingsGroup from '../settings/SettingsGroup.vue'
import SettingsRow from '../settings/SettingsRow.vue'

const props = defineProps<{
  cloud: CloudState;
  overlayStore: OverlayStore
}>()
const emit = defineEmits<{ reset: [operationId: string] }>()
const open = ref(false)
const submitted = ref(false)
const operationId = ref('')
const busy = computed(
  () =>
    props.cloud.state === 'resetting' ||
    (submitted.value &&
      props.cloud.phase !== 'ready' &&
      props.cloud.phase !== 'failed')
)
const phaseLabels = {
  stopping: 'Stopping Cloud tasks…',
  saving: 'Saving home files…',
  rebuilding: 'Rebuilding the system…',
  booting: 'Starting Cloud…',
  ready: 'Cloud is ready.',
  failed: 'Reset failed.'
}
const label = computed(
  () => props.cloud.state === 'resetting' && props.cloud.phase
    ? phaseLabels[props.cloud.phase]
    : ({
      unallocated: 'Starts when you need it',
      off: 'Sleeping',
      booting: 'Starting…',
      running: 'Running',
      saving: 'Saving…',
      resetting: 'Resetting…',
      unavailable: 'Unavailable'
    }[props.cloud.state])
)
function begin() {
  submitted.value = false
  operationId.value = crypto.randomUUID()
  open.value = true
}
function reset() {
  if (busy.value || props.cloud.state === 'unavailable')
    return
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
      >Reset environment</Button>
    </SettingsRow>
    <SettingsRow
      label="Storage limits"
      :description="`System: ${Math.round(cloud.systemBytes / 1024 ** 3)} GiB · Home: ${Math.round(cloud.homeBytes / 1024 ** 3)} GiB`"
    />
    <p v-if="cloud.error" class="px-4 pb-3 text-[13px] text-fg-muted">{{ cloud.error }}</p>
    <Dialog
      :is-open="open"
      :overlay-store="overlayStore"
      label="Reset Cloud environment"
      @close="open = false"
    >
      <div class="flex flex-col gap-4 p-5">
        <h3 class="pr-8 text-[15px] font-medium text-fg-emphasis">Reset Cloud environment</h3>
        <p class="text-[13px] leading-5 text-fg-muted">This stops all your Cloud tasks and replaces installed system packages and system settings. Files in your home directory, including every Cloud project, remain.</p>
        <p
          v-if="submitted && cloud.phase"
          role="status"
          aria-live="polite"
          class="text-[13px] text-fg-body"
        >{{ phaseLabels[cloud.phase] }}</p>
        <InlineError v-if="submitted && cloud.error">{{ cloud.error }}</InlineError>
        <div class="flex justify-end gap-2">
          <Button @click="open = false">{{ submitted ? 'Close' : 'Cancel' }}</Button>
          <Button
            v-if="!submitted || cloud.phase === 'failed'"
            variant="danger"
            :disabled="busy"
            @click="reset"
          >{{ submitted ? 'Retry reset' : 'Reset environment' }}</Button>
        </div>
      </div>
    </Dialog>
  </SettingsGroup>
</template>

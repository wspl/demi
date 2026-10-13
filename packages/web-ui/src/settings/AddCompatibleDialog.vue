<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import TextInput from '../ui/TextInput.vue'
import { endpointProblem } from './provider-defaults'
import SettingsRow from './SettingsRow.vue'
import { WIRE_API_LABELS, type SettingsCompatibleDraft, type SettingsWireApi } from './types'

/**
 * Add Provider › Compatible API, after the protocol is chosen: the label, the
 * endpoint and the key (`product.md` § Models & providers). The endpoint is
 * what makes the entry compatible, so it is the one field that must be
 * given; a label left empty is the protocol's name, and a key left empty is
 * added later on the provider's page. The entry's models are the ones the
 * user adds there.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  wireApi: SettingsWireApi
}>()

const emit = defineEmits<{
  close: []
  add: [draft: SettingsCompatibleDraft]
}>()

const label = ref('')
const baseUrl = ref('')
const apiKey = ref('')

// Each opening starts empty.
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      label.value = ''
      baseUrl.value = ''
      apiKey.value = ''
    }
  },
  { immediate: true },
)

const defaultName = computed(() => `${WIRE_API_LABELS[props.wireApi]} API`)
const canAdd = computed(() => endpointProblem(baseUrl.value) === null)

function add(): void {
  if (canAdd.value) {
    emit('add', {
      wireApi: props.wireApi,
      label: label.value.trim() || defaultName.value,
      baseUrl: baseUrl.value.trim(),
      apiKey: apiKey.value,
    })
  }
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" size="wide" label="Add Compatible API" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add Compatible API</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          A proxy or a server that speaks the {{ WIRE_API_LABELS[wireApi] }} API. It lists the models you add.
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow label="Name">
          <TextInput v-model="label" aria-label="Name" :placeholder="defaultName" focused class="w-64 max-w-full" />
        </SettingsRow>
        <SettingsRow label="Endpoint">
          <TextInput v-model="baseUrl" aria-label="Endpoint" literal trim class="w-64 max-w-full" />
        </SettingsRow>
        <SettingsRow label="API key" description="Stored encrypted on the server. Leave it empty to add it later.">
          <TextInput v-model="apiKey" aria-label="API key" masked="secret" trim class="w-64 max-w-full" />
        </SettingsRow>
      </div>
    </div>
    <template #footer>
      <Button @click="emit('close')">Cancel</Button>
      <Button variant="primary" :disabled="!canAdd" @click="add">Add</Button>
    </template>
  </Dialog>
</template>

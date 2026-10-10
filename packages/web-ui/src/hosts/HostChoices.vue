<script setup lang="ts">
import { Cloud } from '@lucide/vue'
import MenuItem from '../ui/MenuItem.vue'
import type { SentenceText } from '../ui/ui-text'
import { DEVICE_GLYPHS } from '../devices/device-glyphs'
import { DEVICE_STATE_LABEL } from '../devices/state'
import { DEVICE_PATH_LABEL } from '../devices/direct'
import type { HostChoice, HostDeviceOption, HostMenuHost } from './types'

/**
 * The Hosts a conversation can run on, as Run On lists them
 * (`product.md` § Where a conversation runs): the Cloud, then each paired
 * device, an online one with the path this page reaches it by, the primary
 * Host checked. In a project, choosing a device other than the checked one
 * opens its directory picker first, which an ellipsis marks. The header's
 * host menu lists every device, an offline one unavailable; an offline
 * primary Host's card lists the online ones only.
 */
const props = defineProps<{
  primaryHost: HostMenuHost
  devices: HostDeviceOption[]
  /** In a project: choosing a Host opens its directory picker. */
  chooseDirectory?: boolean
  /** Only the Hosts that can be chosen now: the Cloud and the online devices but the primary. */
  onlineOnly?: boolean
  /** Why nothing can be chosen, when nothing can. */
  locked?: SentenceText
}>()
const emit = defineEmits<{
  choose: [host: HostChoice]
}>()

function deviceChecked(id: string): boolean {
  return props.primaryHost.kind === 'device' && props.primaryHost.id === id
}

function listed(device: HostDeviceOption): boolean {
  return !props.onlineOnly || (device.state === 'online' && !deviceChecked(device.id))
}

/** A device's name: with an ellipsis where choosing it opens a picker first. */
function choiceLabel(device: HostDeviceOption): string {
  const opensPicker = props.chooseDirectory && device.state === 'online' && !deviceChecked(device.id)
  return opensPicker ? `${device.name}…` : device.name
}

/** The end of a device's row: the path this page reaches it by while it is online, otherwise its state. */
function deviceNote(device: HostDeviceOption): string | undefined {
  if (device.state !== 'online')
    return DEVICE_STATE_LABEL[device.state]
  return device.path ? DEVICE_PATH_LABEL[device.path] : undefined
}
</script>

<template>
  <MenuItem
    v-if="!onlineOnly || primaryHost.kind !== 'cloud'"
    :icon="Cloud"
    label="Cloud"
    choice
    :is-selected="primaryHost.kind === 'cloud'"
    :disabled="!!locked"
    :disabled-reason="locked"
    @select="emit('choose', { kind: 'cloud' })"
  />
  <template v-for="device in devices" :key="device.id">
    <MenuItem
      v-if="listed(device)"
      :icon="DEVICE_GLYPHS[device.state]"
      :label="choiceLabel(device)"
      :value="deviceNote(device)"
      choice
      :is-selected="deviceChecked(device.id)"
      :disabled="!!locked || device.state !== 'online'"
      :disabled-reason="locked ?? 'This device is offline.'"
      @select="emit('choose', { kind: 'device', id: device.id })"
    />
  </template>
</template>

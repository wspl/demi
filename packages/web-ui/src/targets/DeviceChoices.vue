<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import MenuItem from '../ui/MenuItem.vue'
import type { SentenceText } from '../ui/ui-text'
import { deviceGlyph } from '../devices/device-glyphs'
import { deviceIcon } from './icons'
import { deviceStatusLabel } from '../devices/state'
import { DEVICE_PATH_LABEL } from '../devices/direct'
import type { DeviceChoice, DeviceOption, MenuDevice } from './types'

/**
 * The Devices a conversation can run on, as Run On lists them: the Cloud, then each
 * paired device.
 *
 * - An online device shows the path this page reaches it by, or fainter the path
 *   this browser found last time before the page reached it.
 * - The primary device is checked.
 * - In a project, choosing a device other than the checked one opens its
 *   directory picker first, which an ellipsis marks.
 * - The header's device menu lists every device, an offline one unavailable. An
 *   offline primary device's card lists the online ones only.
 */
const props = defineProps<{
  primaryDevice: MenuDevice
  devices: DeviceOption[]
  /** In a project: choosing a device opens its directory picker. */
  chooseDirectory?: boolean
  /** Only the devices that can be chosen now: the Cloud and the online devices but the primary. */
  onlineOnly?: boolean
  /** Why nothing can be chosen, when nothing can. */
  locked?: SentenceText
}>()
const emit = defineEmits<{
  choose: [device: DeviceChoice]
}>()

function deviceChecked(id: string): boolean {
  return props.primaryDevice.kind === 'device' && props.primaryDevice.id === id
}

function listed(device: DeviceOption): boolean {
  return !props.onlineOnly || (device.state === 'online' && !deviceChecked(device.id))
}

/** A device's name: with an ellipsis where choosing it opens a picker first. */
function choiceLabel(device: DeviceOption): string {
  const opensPicker = props.chooseDirectory && device.state === 'online' && !deviceChecked(device.id)
  return opensPicker ? `${device.name}…` : device.name
}

/**
 * The end of a device's row: the path this page reaches it by while it is
 * online, or the one this browser found last time; otherwise its state, and
 * a download of its next runner while one moves.
 */
function deviceNote(device: DeviceOption): string | undefined {
  if (device.state !== 'online' || device.downloadPercent != null)
    return deviceStatusLabel(device)
  const path = device.path ?? device.lastPath
  return path ? DEVICE_PATH_LABEL[path] : undefined
}

/** Whether the row's path is the one found last time, which may be out of date. */
function lastPathShown(device: DeviceOption): boolean {
  return device.state === 'online' && !device.path && !!device.lastPath
}
</script>

<template>
  <MenuItem
    v-if="!onlineOnly || primaryDevice.kind !== 'cloud'"
    :icon="deviceIcon({ kind: 'cloud' })"
    :label="uiText('common.cloud')"
    choice
    :is-selected="primaryDevice.kind === 'cloud'"
    :disabled="!!locked"
    :disabled-reason="locked"
    @select="emit('choose', { kind: 'cloud' })"
  />
  <template v-for="device in devices" :key="device.id">
    <MenuItem
      v-if="listed(device)"
      :icon="deviceGlyph(device.state, device.mark)"
      :label="choiceLabel(device)"
      :value="deviceNote(device)"
      :value-faint="lastPathShown(device)"
      choice
      :is-selected="deviceChecked(device.id)"
      :disabled="!!locked || device.state !== 'online'"
      :disabled-reason="locked ?? t('choices.offline')"
      @select="emit('choose', { kind: 'device', id: device.id })"
    />
  </template>
</template>

<script setup lang="ts">
import { keptComputed } from '@demicodes/web-ui/infra/keep-same'
import { computed } from 'vue'
import DeviceMenu from '@demicodes/web-ui/targets/DeviceMenu.vue'
import type { DeviceChoice, MenuDevice } from '@demicodes/web-ui/targets/types'
import type { Conversation } from '../state/types'
import { useResources } from '../state/resources'
import { targetKindOf } from '../state/devices'
import type { MoveLock } from '@demicodes/web-ui/targets/move-question'
import { executionFor, deviceOptions, primaryDeviceOf } from './execution'
import { useConversations } from '../conversation/store'

/**
 * The header's device menu over the product's data. Choosing a device is the
 * header's to carry out, which asks before a conversation with messages moves.
 */
const props = defineProps<{
  conversation: Conversation
  locked?: MoveLock
}>()
const emit = defineEmits<{
  choose: [device: DeviceChoice]
}>()
const resources = useResources()
const store = useConversations()

const primaryDevice = keptComputed(() => primaryDeviceOf(executionFor(props.conversation)))
const devices = keptComputed(() => deviceOptions(resources.devices))
const attachedDevices = computed<MenuDevice[]>(() =>
  (props.conversation.attachedDevices.shown.value ?? []).map((attached) => {
    const device = resources.deviceById(attached.deviceId)
    return {
      id: attached.deviceId,
      name: attached.name,
      kind: targetKindOf(device),
      state: device?.state ?? 'offline',
      mark: device?.mark ?? null,
    }
  }),
)
</script>

<template>
  <DeviceMenu
    :primary-device="primaryDevice"
    :conversation-id="conversation.id"
    :pending="store.pendingChanges.includes(conversation.id)"
    :devices="devices"
    :attached-devices="attachedDevices"
    :choose-directory="conversation.target.kind === 'project'"
    :locked="locked"
    @choose="emit('choose', $event)"
    @detach="store.detachDevice(conversation, $event)"
    @connect="resources.openPairing()"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import type { Conversation } from '../state/types'
import { useResources } from '../state/resources'
import { executionFor } from './execution'
import { useConversations } from '../conversation/store'

/**
 * The header's host menu over the product's data (`product.md` § Where a
 * conversation runs). Choosing a Host is the header's to carry out, which
 * asks before a conversation with messages moves.
 */
const props = defineProps<{
  conversation: Conversation
  locked: boolean
}>()
const emit = defineEmits<{
  choose: [host: { kind: 'cloud' } | { kind: 'device'; id: string }]
}>()
const resources = useResources()
const store = useConversations()

const primaryHost = computed<HostMenuHost>(() => {
  const execution = executionFor(props.conversation)
  return {
    id: execution.deviceId ?? 'cloud',
    name: execution.name,
    kind: execution.kind,
    state: execution.state,
  }
})
// The path this page reaches each device by comes with the direct channel's
// state of each device, which this page does not keep yet: until it does, a
// device shows no path.
const devices = computed<HostDeviceOption[]>(() =>
  resources.devices.map((device) => ({ id: device.id, name: device.name, state: device.state })),
)
const attachedHosts = computed<HostMenuHost[]>(() =>
  props.conversation.attachedHosts.map((host) => {
    const device = resources.deviceById(host.deviceId)
    return {
      id: host.deviceId,
      name: host.name,
      kind: device?.kind === 'managed' ? 'cloud' : 'device',
      state: device?.state ?? 'offline',
    }
  }),
)
</script>

<template>
  <HostMenu
    :primary-host="primaryHost"
    :pending="store.pendingChanges.includes(conversation.id)"
    :devices="devices"
    :attached-hosts="attachedHosts"
    :choose-directory="conversation.target.kind === 'workspace'"
    :locked="locked"
    @choose="emit('choose', $event)"
    @detach="store.detachHost(conversation, $event)"
    @connect="resources.openPairing()"
  />
</template>

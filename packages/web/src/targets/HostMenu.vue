<script setup lang="ts">
import { computed } from 'vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import type { HostMenuHost } from '@demicodes/web-ui/hosts/types'
import type { Conversation } from '../state/types'
import { useResources } from '../state/resources'
import { executionFor } from './execution'
import { useConversations } from '../conversation/store'

const props = defineProps<{
  conversation: Conversation
}>()
const emit = defineEmits<{
  switchPrimary: [deviceId: string, cwd?: string | null]
}>()
const resources = useResources()
const store = useConversations()

const primaryLocked = computed(
  () => props.conversation.phase !== 'idle' || props.conversation.archived,
)
const primaryHost = computed<HostMenuHost>(() => {
  const execution = executionFor(props.conversation)
  return {
    id: execution.deviceId ?? 'cloud',
    name: execution.name,
    kind: execution.kind,
    state: execution.state,
  }
})
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

function switchPrimary(id: string) {
  if (primaryLocked.value) {
    return
  }
  const attached = props.conversation.attachedHosts.find(
    (host) => host.deviceId === id,
  )
  emit('switchPrimary', id, attached?.cwd)
}

function attach(id: string) {
  store.attachHost(props.conversation, id)
}

function detach(id: string) {
  store.detachHost(props.conversation, id)
}

function connect() {
  resources.openPairing()
}
</script>

<template>
  <HostMenu
    :primary-host="primaryHost"
    :pending="store.pendingChanges.includes(conversation.id)"
    :attached-hosts="attachedHosts"
    :devices="resources.devices"
    :primary-locked="primaryLocked"
    :attachments-locked="conversation.archived"
    @switch-primary="switchPrimary"
    @attach="attach"
    @detach="detach"
    @connect="connect"
  />
</template>

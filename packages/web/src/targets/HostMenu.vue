<script setup lang="ts">
import { computed } from 'vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import type { HostMenuMainHost } from '@demicodes/web-ui/hosts/types'
import type { Conversation, Project } from '../state/types'
import { useResources } from '../state/resources'
import { executionFor } from './execution'
import { useConversations } from '../conversation/store'

const props = defineProps<{
  conversation: Conversation
  project?: Project
}>()
const emit = defineEmits<{
  switchMain: [deviceId: string, cwd?: string | null]
}>()
const resources = useResources()
const store = useConversations()

const mainLocked = computed(
  () => props.conversation.phase !== 'idle' || props.conversation.archived,
)
const mainHost = computed<HostMenuMainHost>(() => {
  const execution = executionFor(props.conversation)
  return {
    id: execution.deviceId ?? 'cloud',
    name: execution.name,
    kind: execution.kind,
  }
})
const attachedHosts = computed(() =>
  props.conversation.attachedHosts.map((host) => ({
    id: host.deviceId,
    name: host.name,
    online: host.online,
  })),
)

function switchMain(id: string) {
  if (mainLocked.value) {
    return
  }
  const attached = props.conversation.attachedHosts.find(
    (host) => host.deviceId === id,
  )
  emit('switchMain', id, attached?.cwd)
}

function attach(id: string) {
  store.attachHost(props.conversation, id)
}

function detach(id: string) {
  store.detachHost(props.conversation, id)
}

function connect() {
  resources.pairingOpen = true
}
</script>

<template>
  <HostMenu
    :main-host="mainHost"
    :pending="store.pendingChanges.includes(conversation.id)"
    :attached-hosts="attachedHosts"
    :devices="resources.devices"
    :main-locked="mainLocked"
    :attachments-locked="conversation.archived"
    @switch-main="switchMain"
    @attach="attach"
    @detach="detach"
    @connect="connect"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import type { HostMenuMainHost } from '@demicodes/web-ui/hosts/types'
import type { Conversation, Project } from '../prototype/types'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'

const props = defineProps<{
  conversation: Conversation;
  project?: Project
}>()
const emit = defineEmits<{ switchMain: [deviceId: string, cwd?: string] }>()
const resources = useResources()
const store = useConversations()

const mainLocked = computed(() => !!props.conversation.stream || props.conversation.archived)
const mainHost = computed<HostMenuMainHost>(() => ({
  id: props.project?.deviceId ?? 'cloud',
  name: props.project?.host ?? 'Cloud',
  kind: !props.project || props.project.hostKind === 'cloud'
    ? 'cloud'
    : 'device',
}))
const attachedHosts = computed(() => props.conversation.attachedHosts.map(host => ({
  id: host.deviceId,
  name: host.name,
  online: host.deviceId === 'cloud'
    || !!resources.devices.find(device => device.id === host.deviceId)?.online,
})))

function switchMain(id: string) {
  if (mainLocked.value)
    return
  const attached = props.conversation.attachedHosts.find(host => host.deviceId === id)
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

<script setup lang="ts">
import { computed, ref } from 'vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import MoveConversationDialog from '@demicodes/web-ui/hosts/MoveConversationDialog.vue'
import { useMoveQuestion } from '@demicodes/web-ui/hosts/move-question'
import type { WorkToStop } from '@demicodes/web-ui/hosts/move-question'
import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { slow } from '../fixtures/slow'
import { productWould } from '../product-would'

/**
 * A conversation's host menu acting on the specimen's own conversation as
 * the product's header does (`hosts.md` § Where a conversation runs): a
 * conversation with messages asks before it moves, one whose agent works
 * asks to stop it first, a new one moves at once, one in a project opens the chosen device's directory picker, which the
 * gallery says in a toast, and Detach takes the device from Attached.
 */
const props = defineProps<{
  devices: HostDeviceOption[]
  /** Where it runs at first: the Cloud, or a device of `devices`. */
  primaryId: string | null
  attachedIds?: string[]
  /** The conversation is in a project, so a device opens its directory picker. */
  inProject?: boolean
  /** The conversation has messages, so a move asks first. */
  hasMessages?: boolean
  /** The agent works: a move asks to stop it first, naming what stops. */
  working?: WorkToStop
}>()

const homes: Record<string, string> = { mac: '/Users/alex', studio: '/home/alex', build: '/home/ci' }
const primaryId = ref(props.primaryId)
const attachedIds = ref(props.attachedIds ?? [])
const moveQuestion = useMoveQuestion()

function host(id: string | null): HostMenuHost {
  const device = props.devices.find((item) => item.id === id)
  return device
    ? { id: device.id, name: device.name, kind: 'device', state: device.state, mark: device.mark }
    : { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online', mark: null }
}

const primaryHost = computed(() => host(primaryId.value))
const attachedHosts = computed(() => attachedIds.value.map(host))

async function moveTo(id: string | null) {
  const to = host(id)
  const from = primaryHost.value
  if (props.hasMessages || props.working) {
    const home = id ? homes[id] : undefined
    await moveQuestion.ask(
      {
        host: to.name,
        directory: home ? '~' : null,
        from: from.name,
        fromCloud: from.kind === 'cloud',
        ...(props.working ? { stops: props.working } : {}),
      },
      async (tell) => {
        if (props.working) {
          productWould('Stop the Agent')
        }
        if (tell) {
          productWould(`Tell the Agent It Moved to ${to.name}`)
        }
        primaryId.value = id
        return true
      },
    )
    return
  }
  primaryId.value = id
}

function choose(chosen: { kind: 'cloud' } | { kind: 'device'; id: string }) {
  if (chosen.kind === 'cloud') {
    if (primaryHost.value.kind !== 'cloud') {
      void moveTo(null)
    }
    return
  }
  if (chosen.id === primaryId.value) {
    return
  }
  if (props.inProject) {
    productWould(`Open the Directory Picker of ${host(chosen.id).name}`)
    return
  }
  void moveTo(chosen.id)
}

async function detach(id: string) {
  await slow()
  attachedIds.value = attachedIds.value.filter((attached) => attached !== id)
}
</script>

<template>
  <HostMenu
    :primary-host="primaryHost"
    conversation-id="gallery-host-menu"
    :devices="devices"
    :attached-hosts="attachedHosts"
    :choose-directory="inProject"
    @choose="choose"
    @detach="detach"
    @connect="productWould('Add Device')"
  />
  <MoveConversationDialog
    :is-open="moveQuestion.open.value"
    :overlay-store="appOverlayStore"
    :question="moveQuestion.question.value"
    :answers="moveQuestion.answers"
    @close="moveQuestion.cancel"
    @move="moveQuestion.answer"
  />
</template>

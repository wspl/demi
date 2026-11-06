<script setup lang="ts">
import { computed, ref } from 'vue'
import DeviceMenu from '@demicodes/web-ui/targets/DeviceMenu.vue'
import MoveConversationDialog from '@demicodes/web-ui/targets/MoveConversationDialog.vue'
import { useMoveQuestion } from '@demicodes/web-ui/targets/move-question'
import type { DeviceOption, MenuDevice } from '@demicodes/web-ui/targets/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { slow } from '../fixtures/slow'
import { productWould } from '../product-would'

/**
 * A conversation's device menu acting on the specimen's own conversation as the
 * product's header does:
 *
 * - A conversation with messages asks before it moves.
 * - One whose agent works asks to stop it first.
 * - A new one moves at once.
 * - One in a project opens the chosen device's directory picker, which the
 *   gallery says in a toast.
 * - Detach takes the device from Attached.
 */
const props = defineProps<{
  devices: DeviceOption[]
  /** Where it runs at first: the Cloud, or a device of `devices`. */
  primaryId: string | null
  attachedIds?: string[]
  /** The conversation is in a project, so a device opens its directory picker. */
  inProject?: boolean
  /** The conversation has messages, so a move asks first. */
  hasMessages?: boolean
}>()

const homes: Record<string, string> = { mac: '/Users/alex', studio: '/home/alex', build: '/home/ci' }
const primaryId = ref(props.primaryId)
const attachedIds = ref(props.attachedIds ?? [])
const moveQuestion = useMoveQuestion()

function menuDevice(id: string | null): MenuDevice {
  const device = props.devices.find((item) => item.id === id)
  return device
    ? { id: device.id, name: device.name, kind: 'device', state: device.state, downloadPercent: device.downloadPercent, mark: device.mark }
    : { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online', mark: null }
}

const primaryDevice = computed(() => menuDevice(primaryId.value))
const attachedDevices = computed(() => attachedIds.value.map(menuDevice))

async function moveTo(id: string | null) {
  const to = menuDevice(id)
  const from = primaryDevice.value
  if (props.hasMessages) {
    const home = id ? homes[id] : undefined
    await moveQuestion.ask(
      {
        device: to.name,
        directory: home ? '~' : null,
        from: from.name,
        fromCloud: from.kind === 'cloud',
      },
      async () => {
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
    if (primaryDevice.value.kind !== 'cloud') {
      void moveTo(null)
    }
    return
  }
  if (chosen.id === primaryId.value) {
    return
  }
  if (props.inProject) {
    productWould(`Open the Directory Picker of ${menuDevice(chosen.id).name}`)
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
  <DeviceMenu
    :primary-device="primaryDevice"
    conversation-id="gallery-device-menu"
    :devices="devices"
    :attached-devices="attachedDevices"
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

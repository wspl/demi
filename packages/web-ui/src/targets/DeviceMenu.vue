<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref } from 'vue'
import { Plus, Unlink } from '@lucide/vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import MenuGroup from '../ui/MenuGroup.vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import { ICON_PX } from '../ui/icon-metrics'
import { operations, type Action } from '../ui/operations'
import { COMPACT_LABEL_CLASS, useRoomLabel } from '../ui/label-room'
import Tooltip from '../ui/Tooltip.vue'
import { isTextCut } from '../ui/truncation'
import { targetGlyph } from '../devices/device-glyphs'
import { deviceStatusLabel } from '../devices/state'
import DeviceChoices from './DeviceChoices.vue'
import { MOVE_LOCK_REASON, type MoveLock } from './move-question'
import type { DeviceChoice, DeviceOption, MenuDevice } from './types'

/**
 * Where a conversation runs, as the header shows it and its one-level menu
 * changes it.
 *
 * - Run On lists the Cloud and every paired device, each online device with the
 *   path this page reaches it by, the primary device checked.
 * - Attached lists the devices the conversation's agents attached, each with
 *   Detach.
 *
 * Choosing a device is the page's to carry out. A conversation outside a project
 * moves there. One in a project opens that device's directory picker, which
 * `chooseDirectory` marks with an ellipsis on each device but the checked one.
 * The Cloud lists no directories to a page, so it moves the conversation to its
 * own directory there and never opens a picker.
 */
const props = defineProps<{
  primaryDevice: MenuDevice
  /** The user's paired devices, which Run On lists after the Cloud. */
  devices: DeviceOption[]
  attachedDevices: MenuDevice[]
  /** In a project: choosing a device opens its directory picker. */
  chooseDirectory?: boolean
  /** The conversation is archived or being changed: nothing here can move it. Work that runs does not lock it. */
  locked?: MoveLock
  pending?: boolean
  /** The conversation's ID, which names the target of its Detach operations. */
  conversationId: string
  /** Detaches a device from the conversation; the header's button works until it settles. */
  onDetach?: Action<[id: string]>
}>()
const emit = defineEmits<{
  choose: [device: DeviceChoice]
  connect: []
}>()
// The menu closes as Detach is pressed, so the header's button, the row of the conversation's devices, shows the work.
const devicesTarget = computed(() => `devices:${props.conversationId}`)
const detaching = computed(() => operations.onTarget(devicesTarget.value))

/** The download of the primary device's next runner while one moves, which the button says beside its name. */
const downloading = computed(() =>
  props.primaryDevice.kind === 'device' && props.primaryDevice.downloadPercent != null
    ? deviceStatusLabel(props.primaryDevice)
    : null,
)

const open = ref(false)
// Where the header's title needs the width, the device is its icon and its name a tooltip.
const deviceLabel = ref<HTMLElement | null>(null)
const deviceCompact = useRoomLabel(deviceLabel, () => props.primaryDevice.name, 0)

/** The name is in the tooltip while the button hides it or cuts it. */
function nameHidden(): boolean {
  return deviceCompact.value || (deviceLabel.value != null && isTextCut(deviceLabel.value))
}


function choose(device: DeviceChoice) {
  if (props.locked)
    return
  open.value = false
  emit('choose', device)
}

/**
 * An attached device chosen to run on, as in Run On. An offline device is
 * drawn faded and cannot be chosen, but its row stays enabled, since Detach
 * takes it back whatever its state.
 */
function chooseAttached(device: MenuDevice) {
  if (device.kind === 'cloud') {
    choose({ kind: 'cloud' })
  } else if (device.state === 'online') {
    choose({ kind: 'device', id: device.id })
  }
}

function detach(id: string): unknown {
  open.value = false
  return props.onDetach?.(id)
}

function connect() {
  open.value = false
  emit('connect')
}
</script>

<template>
  <Dropdown
    v-model:open="open"
    :overlay-store="appOverlayStore"
    :disabled="pending"
    width="shrink"
  >
    <template #trigger>
      <Tooltip :content="primaryDevice.name" :show-if="nameHidden">
        <!-- data-device-menu: the product checks choose a device through it in any language (`browse.md`). -->
        <Button
          variant="ghost"
          class="max-w-full"
          data-device-menu
          :aria-label="t('menu.where')"
          :loading="pending || detaching"
        >
          <component :is="targetGlyph(primaryDevice)" :size="ICON_PX.in28" class="shrink-0" />
          <!-- Wide enough for a Windows computer's default name, DESKTOP- and seven characters. -->
          <span
            ref="deviceLabel"
            class="max-w-36 truncate leading-5"
            :class="deviceCompact ? COMPACT_LABEL_CLASS : ''"
          >{{ primaryDevice.name }}</span>
          <span v-if="downloading" class="text-caption text-fg-faint whitespace-nowrap">
            {{ downloading }}
          </span>
          <span v-if="attachedDevices.length" class="text-caption text-fg-faint">
            +{{ attachedDevices.length }}
          </span>
        </Button>
      </Tooltip>
    </template>
    <template #content>
      <!-- Room for a device's whole name beside a download's note, Downloading update · 40%. -->
      <Menu class="max-w-104">
        <MenuGroup :label="t('menu.runOn')">
          <DeviceChoices
            :primary-device="primaryDevice"
            :devices="devices"
            :choose-directory="chooseDirectory"
            :locked="locked ? MOVE_LOCK_REASON[locked] : undefined"
            @choose="choose"
          />
          <MenuItem :label="t('menu.addDevice')" :icon="Plus" @select="connect" />
        </MenuGroup>
        <MenuGroup v-if="attachedDevices.length" :label="t('menu.attached')">
          <MenuItem
            v-for="device in attachedDevices"
            :key="device.id"
            :icon="targetGlyph(device)"
            :label="device.name"
            :faded="device.kind === 'device' && device.state !== 'online'"
            :disabled="!!locked"
            :disabled-reason="locked ? MOVE_LOCK_REASON[locked] : undefined"
            @select="chooseAttached(device)"
          >
            <template #actions>
              <Tooltip :content="t('menu.detach')" class="inline-flex">
                <IconButton
                  :icon="Unlink"
                  variant="ghost"
                  size="xs"
                  :aria-label="t('menu.detach')"
                  :disabled="!!locked"
                  :operation="{ action: `detach:${device.id}`, target: devicesTarget }"
                  @click.stop="detach(device.id)"
                />
              </Tooltip>
            </template>
          </MenuItem>
        </MenuGroup>
      </Menu>
    </template>
  </Dropdown>
</template>

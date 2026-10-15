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
import { hostGlyph } from '../devices/device-glyphs'
import HostChoices from './HostChoices.vue'
import { MOVE_LOCK_REASON, type MoveLock } from './move-question'
import type { HostChoice, HostDeviceOption, HostMenuHost } from './types'

/**
 * Where a conversation runs, as the header shows it and its one-level menu
 * changes it (`product.md` § Where a conversation runs): Run On lists the
 * Cloud and every paired device, each online device with the path this page
 * reaches it by, the primary Host checked; Attached lists the devices the
 * conversation's agents attached, each with Detach. Choosing a Host is the
 * host's to carry out: a conversation outside a project moves there, one in
 * a project opens that device's directory picker, which `chooseDirectory`
 * marks with an ellipsis on each device but the checked one. The Cloud
 * lists no directories to a page, so it moves the conversation to its own
 * directory there and never opens a picker.
 */
const props = defineProps<{
  primaryHost: HostMenuHost
  /** The user's paired devices, which Run On lists after the Cloud. */
  devices: HostDeviceOption[]
  attachedHosts: HostMenuHost[]
  /** In a project: choosing a Host opens its directory picker. */
  chooseDirectory?: boolean
  /** The conversation is archived or being changed: nothing here can move it. Work that runs does not lock it. */
  locked?: MoveLock
  pending?: boolean
  /** The conversation's ID, which names the target of its Detach operations. */
  conversationId: string
  /** Detaches a Host from the conversation; the header's button works until it settles. */
  onDetach?: Action<[id: string]>
}>()
const emit = defineEmits<{
  choose: [host: HostChoice]
  connect: []
}>()
// The menu closes as Detach is pressed, so the header's button, the row of the conversation's Hosts, shows the work.
const hostsTarget = computed(() => `hosts:${props.conversationId}`)
const detaching = computed(() => operations.onTarget(hostsTarget.value))

const open = ref(false)
// Where the header's title needs the width, the host is its icon and its name a tooltip.
const hostLabel = ref<HTMLElement | null>(null)
const hostCompact = useRoomLabel(hostLabel, () => props.primaryHost.name, 0)

/** The name is in the tooltip while the button hides it or cuts it. */
function nameHidden(): boolean {
  return hostCompact.value || (hostLabel.value != null && isTextCut(hostLabel.value))
}


function choose(host: HostChoice) {
  if (props.locked)
    return
  open.value = false
  emit('choose', host)
}

/**
 * An attached Host chosen to run on, as in Run On. An offline device is
 * drawn faded and cannot be chosen, but its row stays enabled, since Detach
 * takes it back whatever its state.
 */
function chooseAttached(host: HostMenuHost) {
  if (host.kind === 'cloud') {
    choose({ kind: 'cloud' })
  } else if (host.state === 'online') {
    choose({ kind: 'device', id: host.id })
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
      <Tooltip :content="primaryHost.name" :show-if="nameHidden">
        <Button
          variant="ghost"
          class="max-w-full"
          :aria-label="t('menu.where')"
          :loading="pending || detaching"
        >
          <component :is="hostGlyph(primaryHost)" :size="ICON_PX.in28" class="shrink-0" />
          <!-- Wide enough for a Windows computer's default name, DESKTOP- and seven characters. -->
          <span
            ref="hostLabel"
            class="max-w-36 truncate"
            :class="hostCompact ? COMPACT_LABEL_CLASS : ''"
          >{{ primaryHost.name }}</span>
          <span v-if="attachedHosts.length" class="text-[11px] text-fg-faint">
            +{{ attachedHosts.length }}
          </span>
        </Button>
      </Tooltip>
    </template>
    <template #content>
      <Menu class="max-w-80">
        <MenuGroup :label="t('menu.runOn')">
          <HostChoices
            :primary-host="primaryHost"
            :devices="devices"
            :choose-directory="chooseDirectory"
            :locked="locked ? MOVE_LOCK_REASON[locked] : undefined"
            @choose="choose"
          />
          <MenuItem :label="t('menu.addDevice')" :icon="Plus" @select="connect" />
        </MenuGroup>
        <MenuGroup v-if="attachedHosts.length" :label="t('menu.attached')">
          <MenuItem
            v-for="host in attachedHosts"
            :key="host.id"
            :icon="hostGlyph(host)"
            :label="host.name"
            :faded="host.kind === 'device' && host.state !== 'online'"
            :disabled="!!locked"
            :disabled-reason="locked ? MOVE_LOCK_REASON[locked] : undefined"
            @select="chooseAttached(host)"
          >
            <template #actions>
              <Tooltip :content="t('menu.detach')" class="inline-flex">
                <IconButton
                  :icon="Unlink"
                  variant="ghost"
                  size="xs"
                  :aria-label="t('menu.detach')"
                  :disabled="!!locked"
                  :operation="{ action: `detach:${host.id}`, target: hostsTarget }"
                  @click.stop="detach(host.id)"
                />
              </Tooltip>
            </template>
          </MenuItem>
        </MenuGroup>
      </Menu>
    </template>
  </Dropdown>
</template>

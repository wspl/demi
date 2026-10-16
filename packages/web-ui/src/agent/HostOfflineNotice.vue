<script setup lang="ts">
import { t } from './locales/text'
import { ref } from 'vue'
import Button from '../ui/Button.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import DeviceStartHint from '../devices/DeviceStartHint.vue'
import type { DeviceStart } from '../devices/installation'
import HostChoices from '../hosts/HostChoices.vue'
import { MOVE_LOCK_REASON, type MoveLock } from '../hosts/move-question'
import type { HostChoice, HostDeviceOption, HostMenuHost } from '../hosts/types'

/**
 * The conversation's primary Host is a paired device whose runner is not
 * connected (`product.md` § Recovering an unfinished turn): offered above
 * the composer with how to start the runner again, and Move to Another
 * Host…, which lists Run On's online Hosts once the conversation's work
 * is idle; choosing one is the host's to carry out, as the header's host
 * menu's choice is. Gone once the device is online. The row owns the
 * button's inset from its edges.
 */
defineProps<{
  /** The device's name. */
  name: string
  start: DeviceStart
  /** The offline device, checked in Run On. */
  primaryHost: HostMenuHost
  /** The user's paired devices; the card lists the online ones. */
  devices: HostDeviceOption[]
  /** In a project: choosing a device opens its directory picker. */
  chooseDirectory?: boolean
  /** Why the conversation cannot move now: archived, or a move under way. */
  locked?: MoveLock
}>()
const emit = defineEmits<{
  move: [host: HostChoice]
}>()

const open = ref(false)

function choose(host: HostChoice): void {
  open.value = false
  emit('move', host)
}
</script>

<template>
  <div
    class="host-offline-notice flex w-full flex-col rounded-lg bg-surface-card text-chrome text-fg-muted"
    role="status"
  >
    <div class="host-offline-notice-row flex items-center gap-2 pl-3">
      <span class="min-w-0 flex-1 truncate">{{ t('notices.hostOffline', { name }) }}</span>
      <span class="host-offline-notice-move">
        <Dropdown
          v-model:open="open"
          :overlay-store="appOverlayStore"
          :disabled="!!locked"
          :disabled-reason="locked ? MOVE_LOCK_REASON[locked] : undefined"
        >
          <template #trigger>
            <Button size="sm" :disabled="!!locked">{{ t('notices.moveHost') }}</Button>
          </template>
          <template #content>
            <Menu class="max-w-80">
              <HostChoices
                :primary-host="primaryHost"
                :devices="devices"
                :choose-directory="chooseDirectory"
                online-only
                @choose="choose"
              />
            </Menu>
          </template>
        </Dropdown>
      </span>
    </div>
    <DeviceStartHint class="px-3 pb-3 text-detail leading-4 text-fg-subtle" :start="start" />
  </div>
</template>

<style scoped>
.host-offline-notice-row {
  --host-offline-row-h: 36px;
  height: var(--host-offline-row-h);
}

/* The button sits as far from the card's right edge as centering puts it
   from the row's top and bottom: (row height - control height) / 2. */
.host-offline-notice-move {
  display: flex;
  flex-shrink: 0;
  /* No inline strut, so the Dropdown's wrapper cannot lift the control off center. */
  line-height: 0;
  margin-right: calc((var(--host-offline-row-h) - var(--spacing-hit-sm)) / 2);
}

/* A phone's row is 44 px, around its 36 px buttons. */
@media (max-width: 767px) {
  .host-offline-notice-row {
    --host-offline-row-h: var(--spacing-row);
  }
}
</style>

<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref } from 'vue'
import Button from '../ui/Button.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import DeviceStartHint from '../devices/DeviceStartHint.vue'
import type { DeviceStart } from '../devices/installation'
import HostChoices from '../hosts/HostChoices.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import { MOVE_LOCK_REASON, type MoveLock } from '../hosts/move-question'
import type { HostChoice, HostDeviceOption, HostMenuHost } from '../hosts/types'

/**
 * The notice that the conversation's primary Host is a paired device whose
 * runner is not connected.
 *
 * It is offered above the composer, with:
 *
 * - how long ago its runner was last connected, as Settings › Devices says
 *   it, so the user tells a device that dropped a moment ago from one that has
 *   been away for days
 * - how to start the runner again
 * - Move to Another Host…, which lists Run On's online Hosts once the
 *   conversation's work is idle. Choosing one is the host's to carry out, as
 *   the header's host menu's choice is.
 *
 * The notice is gone once the device is online. The row owns the button's inset
 * from its edges.
 *
 * On a phone, the sentence is never cut for the button. When the two do not fit
 * on one line, the sentence wraps and the button moves under it, at its leading
 * edge, as iOS's inline notices keep their text whole.
 */
const props = defineProps<{
  name: string
  /** When its runner was last connected; none for one that never connected. */
  seen?: string
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

/** Where the device's name and the time stand in the sentence, which splits at them. */
const NAME = '\u0000'
const TIME = '\u0001'

/**
 * The sentence as the language words it, in its words, the name and the time,
 * so that only the name gives up its width when the row is too narrow, and the
 * time is always read.
 */
const sentence = computed(() =>
  (props.seen
    ? t('notices.hostOfflineLastSeen', { name: NAME, time: TIME })
    : t('notices.hostOffline', { name: NAME })
  ).split(/(\u0000|\u0001)/).filter((part) => part !== ''))

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
      <span class="host-offline-notice-sentence flex min-w-0 flex-1 items-baseline">
        <template v-for="(part, index) in sentence" :key="index">
          <span v-if="part === NAME" class="host-offline-notice-name min-w-0 truncate">{{ name }}</span>
          <!-- The card is a status; the time ticking each minute is not news to announce again. -->
          <span v-else-if="part === TIME && seen" class="shrink-0 whitespace-pre-wrap" aria-live="off"><RelativeTime :timestamp="seen" /></span>
          <span v-else class="shrink-0 whitespace-pre-wrap">{{ part }}</span>
        </template>
      </span>
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
  /*
   * No inline strut, so the Dropdown's wrapper cannot lift the control off
   * center.
   */
  line-height: 0;
  margin-right: calc((var(--host-offline-row-h) - var(--spacing-hit-sm)) / 2);
}

/* A phone's row is at least 44 px, around its 36 px button, and grows with
   the sentence. The sentence keeps its whole width before the button takes
   any: as wide as its text, it pushes the button onto a line of its own,
   where the button starts under the sentence's first letter. Its own padding
   makes a line of it as tall as the button, so a wrapped row keeps the same
   inset above the text as beside it. */
@media (max-width: 767px) {
  .host-offline-notice-row {
    --host-offline-row-h: var(--spacing-row);
    --host-offline-row-pad: calc((var(--host-offline-row-h) - var(--spacing-hit-sm)) / 2);
    height: auto;
    min-height: var(--host-offline-row-h);
    flex-wrap: wrap;
    row-gap: 0;
    padding-block: var(--host-offline-row-pad);
  }

  /* The sentence is text that wraps, its name too, never a row of parts. */
  .host-offline-notice-sentence {
    display: block;
    flex: 1 1 auto;
    padding-block: calc((var(--spacing-hit-sm) - 1lh) / 2);
  }

  .host-offline-notice-name {
    overflow: visible;
    white-space: normal;
    overflow-wrap: anywhere;
  }
}
</style>

<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import DeviceIcon from '@demicodes/web-ui/devices/DeviceIcon.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import { hostIcon } from '@demicodes/web-ui/hosts/icons'
import { deviceStatusLabel } from '@demicodes/web-ui/devices/state'
import type { SidebarDevice } from './types'

/**
 * Where a row's work runs, as the row's end shows it: a conversation's and a project's row end in
 * this one mark, in the same box, so the devices line up down both groups
 * (`conversations.md` § Conversations and projects). A device is its mark (`DeviceIcon`), the plain
 * computer without a dot once it is removed; its tip and its accessible label are the same text, "MacBook
 * Pro · Online" or "Removed device". A project on the Cloud shows the Cloud's glyph in the same box.
 * The mark stays while the pointer is over the row, so the row's buttons stand to its left.
 */
const props = defineProps<{
  host: SidebarDevice | { kind: 'cloud' }
}>()

const phone = usePhone()

const label = computed(() => {
  const host = props.host
  if (host.kind === 'cloud') {
    return t('device.cloud')
  }
  return host.kind === 'paired' ? `${host.name} · ${deviceStatusLabel(host)}` : t('device.removed')
})
</script>

<template>
  <Tooltip
    :content="label"
    tag="span"
    class="flex size-hit-sm shrink-0 items-center justify-center text-fg-subtle phone:size-8"
  >
    <component
      :is="hostIcon({ kind: 'cloud' })"
      v-if="host.kind === 'cloud'"
      role="img"
      :aria-label="label"
      :size="phone ? ICON_PX.in32 : ICON_PX.in20"
    />
    <DeviceIcon
      v-else-if="host.kind === 'paired'"
      :state="host.state"
      :mark="host.mark"
      :label="label"
    />
    <DeviceIcon v-else :state="null" :mark="null" :label="label" />
  </Tooltip>
</template>

<script setup lang="ts">
import CornerDot from '../ui/CornerDot.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { DEVICE_ICON } from '../hosts/icons'
import { DEVICE_STATE_LABEL, DEVICE_STATE_TONE, type DeviceState } from './state'

/**
 * A device: the product's one device glyph with its state's dot on the corner, the same, at one
 * size, in the devices list, the host menu and the sidebar. A device since removed has no state,
 * and no dot.
 * Without `label` the dot names the state for assistive technology; with it, the whole icon is
 * named, as a mark that stands for the device does ("MacBook Pro · Online").
 */
defineProps<{
  state: DeviceState | null
  label?: string
}>()
</script>

<template>
  <span class="relative flex shrink-0" :role="label ? 'img' : undefined" :aria-label="label">
    <component :is="DEVICE_ICON" :size="ICON_PX.in28" aria-hidden="true" />
    <CornerDot
      v-if="state"
      :tone="DEVICE_STATE_TONE[state]"
      :label="label ? undefined : DEVICE_STATE_LABEL[state]"
    />
  </span>
</template>

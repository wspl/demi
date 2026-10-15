<script setup lang="ts">
import CornerDot from '../ui/CornerDot.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { DEVICE_ICON } from '../hosts/icons'
import DeviceLogo from './DeviceLogo.vue'
import type { DeviceMark } from './mark'
import { DEVICE_STATE_LABEL, DEVICE_STATE_TONE, type DeviceState } from './state'

/**
 * A device's mark (`product.md` § Conversations and projects), the same wherever a device is
 * drawn: its system's logo in the subtle icon color, at a brand mark's size, its state's dot on
 * the logo's top right, and under the dot the number that tells it from the user's other devices
 * of the same logo, when `mark` shows one (`shownMark`). A device since removed has no mark: the
 * plain computer glyph, without a dot, whatever state its caller still holds for it. The dot and the number are cut out of what
 * is painted behind them by an outline in that color (`--fill-color`), as `CornerPlus` is.
 * Without `label` the dot names the state for assistive technology; with it, the whole mark is
 * named, as a mark that stands for the device does ("MacBook Pro · Online"); the logo and the
 * number only help the eye.
 */
defineProps<{
  state: DeviceState | null
  mark: DeviceMark | null
  label?: string
}>()
</script>

<template>
  <span
    class="relative flex size-4 shrink-0 items-center justify-center text-fg-subtle"
    :role="label ? 'img' : undefined"
    :aria-label="label"
  >
    <DeviceLogo v-if="mark" :logo="mark.logo" :size="ICON_PX.markIn28" />
    <component :is="DEVICE_ICON" v-else :size="ICON_PX.in28" aria-hidden="true" />
    <!-- The device's state is a fact about where the work runs, not the work's own state:
      it is the small dot, so a row's status dot beside it is never mistaken for it. -->
    <CornerDot
      v-if="state && mark"
      size="xs"
      :tone="DEVICE_STATE_TONE[state]"
      :label="label ? undefined : DEVICE_STATE_LABEL[state]"
    />
    <!-- Its box centered under the dot; a second digit reaches further left. -->
    <span
      v-if="mark?.number != null"
      class="device-mark-number pointer-events-none absolute -bottom-0.5 -right-0.5 min-w-1.5 text-center text-[8px] font-bold leading-none"
      aria-hidden="true"
    >{{ mark.number }}</span>
  </span>
</template>

<style scoped>
.device-mark-number {
  -webkit-text-stroke: 2px var(--fill-color);
  paint-order: stroke fill;
}
</style>

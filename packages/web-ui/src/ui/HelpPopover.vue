<script setup lang="ts">
import { CircleQuestionMark } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Dropdown from './Dropdown.vue'
import IconButton from './IconButton.vue'
import Tooltip from './Tooltip.vue'
import type { TitleText } from './ui-text'

/**
 * A `?` button that opens help about the thing beside it in a popover: the
 * button's tooltip and accessible name are `label`, the popover's content
 * the default slot. Escape or a click outside closes it, as the button does
 * a second time. The caller places the button where controls go, such as a
 * settings row's controls; the popover owns its padding.
 */
defineProps<{
  label: TitleText
  overlayStore: OverlayStore
}>()
</script>

<template>
  <Dropdown :overlay-store="overlayStore">
    <template #trigger="{ isOpen }">
      <Tooltip :content="label" :disabled="isOpen">
        <IconButton
          :icon="CircleQuestionMark"
          size="sm"
          variant="ghost"
          :aria-label="label"
          :aria-expanded="isOpen"
          :pressed="isOpen"
        />
      </Tooltip>
    </template>
    <template #content>
      <div class="overlay-shell w-96 max-w-[calc(100vw-2rem)] rounded-lg p-3 text-chrome text-fg" role="dialog" :aria-label="label">
        <slot />
      </div>
    </template>
  </Dropdown>
</template>

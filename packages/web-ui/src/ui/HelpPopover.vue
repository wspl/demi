<script setup lang="ts">
import { ref } from 'vue'
import { CircleQuestionMark } from '@lucide/vue'
import { useEventListener } from '@vueuse/core'
import type { OverlayStore } from '../overlay/overlayStore'
import IconButton from './IconButton.vue'
import Popover from './Popover.vue'
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

const open = ref(false)
const anchor = ref<HTMLElement | null>(null)

// Escape closes the popover alone: caught before a dialog it opened in
// hears it, which would close the dialog too.
useEventListener(
  window,
  'keydown',
  (event: KeyboardEvent) => {
    if (!open.value || event.key !== 'Escape')
      return
    event.preventDefault()
    event.stopPropagation()
    open.value = false
  },
  { capture: true },
)
</script>

<template>
  <span ref="anchor" class="inline-flex">
    <Tooltip :content="label" :disabled="open">
      <IconButton
        :icon="CircleQuestionMark"
        size="sm"
        variant="ghost"
        :aria-label="label"
        :aria-expanded="open"
        :pressed="open"
        @click="open = !open"
      />
    </Tooltip>
  </span>
  <Popover
    :is-open="open"
    :overlay-store="overlayStore"
    :anchor-el="anchor"
    placement="bottom-end"
    :ignore-els="anchor ? [anchor] : []"
    @close="open = false"
  >
    <div class="overlay-shell w-96 max-w-[calc(100vw-2rem)] rounded-lg p-3 text-chrome text-fg" role="dialog" :aria-label="label">
      <slot />
    </div>
  </Popover>
</template>

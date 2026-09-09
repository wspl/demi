<script setup lang="ts">
import { computed, provide, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { disabledTooltip } from './disabled'
import Popover from './Popover.vue'
import DropdownTrigger from './DropdownTrigger.vue'
import type { DropdownSize, DropdownVariant } from './DropdownTrigger.vue'
import { menuRootKey } from './menu-context'
import Tooltip from './Tooltip.vue'

const props = withDefaults(defineProps<{
  overlayStore: OverlayStore
  placement?: 'top' | 'top-start' | 'top-end' | 'bottom' | 'bottom-start' | 'bottom-end'
  offset?: number
  anchorInset?: number
  shiftPadding?: number
  variant?: DropdownVariant
  /** Accessible name for a built-in trigger whose visible text is a value, not a label. */
  triggerLabel?: string
  size?: DropdownSize
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
}>(), {
  placement: 'bottom-start',
  offset: 4,
  size: 'md',
})

const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))

const emit = defineEmits<{
  close: []
}>()

const isOpen = defineModel<boolean>('open', { default: false })
const triggerRef = ref<HTMLDivElement>()
const ignoreEls = computed(() => triggerRef.value ? [triggerRef.value] : [])
const triggerWidth = computed(() => {
  const el = triggerRef.value
  if (!el)
    return 0
  const inset = props.anchorInset ?? 0
  return Math.max(0, el.getBoundingClientRect().width - inset * 2)
})

function handleClick() {
  if (props.disabled)
    return
  isOpen.value = !isOpen.value
}

function open() {
  if (props.disabled)
    return
  isOpen.value = true
}

function close() {
  if (!isOpen.value)
    return
  isOpen.value = false
  emit('close')
}

provide(menuRootKey, { dismiss: close })

defineExpose({ open, close })
</script>

<template>
  <Tooltip
    :content="tooltipContent"
    :disabled="!tooltipContent"
    tag="div"
    class="relative inline-flex"
    :open-delay-ms="80"
  >
    <div class="relative inline-flex">
      <div
        ref="triggerRef"
        class="cursor-default"
        @click="handleClick"
      >
        <DropdownTrigger
          v-if="props.variant"
          :is-open="isOpen"
          :variant="props.variant"
          :size="props.size"
          :aria-label="props.triggerLabel"
          :disabled="props.disabled"
        >
          <slot name="trigger" :is-open="isOpen" />
        </DropdownTrigger>
        <slot
          v-else
          name="trigger"
          :is-open="isOpen"
        />
      </div>
      <Popover
        :overlay-store="props.overlayStore"
        :is-open="isOpen"
        :anchor-el="triggerRef"
        :anchor-inset="props.anchorInset"
        :placement="props.placement"
        :offset="props.offset"
        :shift-padding="props.shiftPadding"
        :ignore-els="ignoreEls"
        @close="close"
      >
        <slot
          name="content"
          :close="close"
          :trigger-width="triggerWidth"
        />
      </Popover>
    </div>
  </Tooltip>
</template>

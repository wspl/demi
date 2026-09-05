<script setup lang="ts">
import { computed, provide, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Popover from './Popover.vue'
import DropdownTrigger from './DropdownTrigger.vue'
import type { DropdownSize, DropdownVariant } from './DropdownTrigger.vue'
import { menuRootKey } from './menu-context'

const props = withDefaults(defineProps<{
  overlayStore: OverlayStore
  placement?: 'top' | 'top-start' | 'top-end' | 'bottom' | 'bottom-start' | 'bottom-end'
  offset?: number
  anchorInset?: number
  shiftPadding?: number
  variant?: DropdownVariant
  size?: DropdownSize
}>(), {
  placement: 'bottom-start',
  offset: 4,
  size: 'md',
})

const emit = defineEmits<{
  close: []
}>()

const isOpen = defineModel<boolean>('open', { default: false })
const triggerRef = ref<HTMLDivElement>()
const ignoreEls = computed(() => triggerRef.value ? [triggerRef.value] : [])
const triggerWidth = computed(() => {
  const el = triggerRef.value
  if (!el) return 0
  const inset = props.anchorInset ?? 0
  return Math.max(0, el.getBoundingClientRect().width - inset * 2)
})

function handleClick() {
  isOpen.value = !isOpen.value
}

function open() {
  isOpen.value = true
}

function close() {
  if (!isOpen.value) return
  isOpen.value = false
  emit('close')
}

provide(menuRootKey, { dismiss: close })

defineExpose({ open, close })
</script>

<template>
  <div class="relative inline-flex">
    <div ref="triggerRef" class="cursor-default" @click="handleClick">
      <DropdownTrigger v-if="props.variant" :is-open="isOpen" :variant="props.variant" :size="props.size">
        <slot name="trigger" :is-open="isOpen" />
      </DropdownTrigger>
      <slot v-else name="trigger" :is-open="isOpen" />
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
      <slot name="content" :close="close" :trigger-width="triggerWidth" />
    </Popover>
  </div>
</template>

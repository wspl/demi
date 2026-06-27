<script setup lang="ts">
import { computed, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Popover from './Popover.vue'

const props = withDefaults(defineProps<{
  overlayStore: OverlayStore
  placement?: 'top' | 'top-start' | 'top-end' | 'bottom' | 'bottom-start' | 'bottom-end'
  offset?: number
  anchorInset?: number
  shiftPadding?: number
}>(), {
  placement: 'top',
  offset: 8,
})

const emit = defineEmits<{
  close: []
}>()

const isOpen = ref(false)
const anchorX = ref(0)
const anchorY = ref(0)
const anchorWidth = ref(0)
const anchorHeight = ref(0)
const triggerRef = ref<HTMLDivElement>()
const ignoreEls = computed(() => triggerRef.value ? [triggerRef.value] : [])

function updateAnchor() {
  if (!triggerRef.value) return
  const rect = triggerRef.value.getBoundingClientRect()
  const inset = props.anchorInset ?? 0
  anchorX.value = rect.left + inset
  anchorY.value = rect.top
  anchorWidth.value = rect.width - inset * 2
  anchorHeight.value = rect.height
}

function handleClick() {
  updateAnchor()
  isOpen.value = !isOpen.value
}

function open() {
  updateAnchor()
  isOpen.value = true
}

function close() {
  if (!isOpen.value) return
  isOpen.value = false
  emit('close')
}

defineExpose({ open, close })
</script>

<template>
  <div ref="triggerRef" @click="handleClick">
    <slot name="trigger" :is-open="isOpen" />
  </div>
  <Popover
    :overlay-store="props.overlayStore"
    :is-open="isOpen"
    :anchor-x="anchorX"
    :anchor-y="anchorY"
    :anchor-width="anchorWidth"
    :anchor-height="anchorHeight"
    :placement="props.placement"
    :offset="props.offset"
    :shift-padding="props.shiftPadding"
    :ignore-els="ignoreEls"
    @close="close"
  >
    <slot name="content" :close="close" :trigger-width="anchorWidth" />
  </Popover>
</template>

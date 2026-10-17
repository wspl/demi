<script setup lang="ts">
import { computed } from 'vue'
import { deviceOutline, type OutlineBox, type OutlineButton } from './outline'

/**
 * A device's wireframe frame around its screen (`mobile.md` § The live
 * view): a thin outline with the screen's corners and its buttons as short
 * marks, drawn over the view's area, through which the screen's input
 * passes.
 */
const props = defineProps<{
  /** The screen's box in the area, in CSS pixels. */
  screen: OutlineBox
  /** The screen's corner radius, in CSS pixels. */
  cornerRadius: number
  buttons: readonly OutlineButton[]
}>()

const shape = computed(() => deviceOutline(props.screen, props.cornerRadius, props.buttons))
</script>

<template>
  <div class="pointer-events-none absolute inset-0" aria-hidden="true">
    <div
      class="absolute border border-fg-faint"
      :style="{
        left: `${shape.body.left}px`,
        top: `${shape.body.top}px`,
        width: `${shape.body.width}px`,
        height: `${shape.body.height}px`,
        borderRadius: `${shape.body.radius}px`,
      }"
    />
    <div
      v-for="(mark, index) in shape.marks"
      :key="index"
      class="absolute rounded-full bg-fg-faint"
      :style="{ left: `${mark.left}px`, top: `${mark.top}px`, width: `${mark.width}px`, height: `${mark.height}px` }"
    />
  </div>
</template>

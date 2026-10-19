<script setup lang="ts">
import { computed } from 'vue'
import { drawnFrame, type FrameBox, type FrameButton, type ScreenCorners } from './drawn-frame'

/**
 * A device's frame drawn by the view (`mobile.md` § The live view), in the
 * colors of the frames Xcode installs, which are the same in the light and
 * the dark appearance: a black body around its screen, an even bezel
 * concentric with the screen's corners, a dark grey metal band along its
 * outside with a lit edge, which keeps the body's outline visible on a dark
 * background, and its buttons as short bars of the band's grey, a shade
 * lighter at their ends. It lies under the screen's picture, and the
 * screen's input passes through it.
 */
const props = defineProps<{
  /** The screen's box in the area, in CSS pixels. */
  screen: FrameBox
  /** The screen's corners, in CSS pixels. */
  corners: ScreenCorners
  buttons: readonly FrameButton[]
}>()

const shape = computed(() => drawnFrame(props.screen, props.corners, props.buttons))

/** Xcode's frames' colors, sampled from the frame of an iPhone 17 Pro as Demi draws it. */
const BODY = '#000000'
const METAL = '#2c2c2c'
const LIT = '#787878'
const BUTTON_END = '#3e3e3e'

/** A button's shading: the band's grey, a shade lighter at its two ends. */
function shading(side: FrameButton['side']): string {
  const along = side === 'left' || side === 'right' ? 'to bottom' : 'to right'
  return `linear-gradient(${along}, ${BUTTON_END}, ${METAL} 25%, ${METAL} 75%, ${BUTTON_END})`
}
</script>

<template>
  <div class="pointer-events-none absolute inset-0" aria-hidden="true">
    <div
      v-for="(mark, index) in shape.marks"
      :key="index"
      class="absolute"
      :style="{
        left: `${mark.left}px`,
        top: `${mark.top}px`,
        width: `${mark.width}px`,
        height: `${mark.height}px`,
        background: shading(mark.side),
        borderRadius: `${Math.min(mark.width, mark.height) / 2}px`,
      }"
    />
    <svg class="absolute inset-0 h-full w-full overflow-visible">
      <path :d="shape.body" :fill="BODY" />
      <path :d="shape.band" fill="none" :stroke="METAL" :stroke-width="shape.bandWidth" />
      <path :d="shape.edge" fill="none" :stroke="LIT" :stroke-width="shape.edgeWidth" />
    </svg>
  </div>
</template>

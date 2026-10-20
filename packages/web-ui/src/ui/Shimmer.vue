<script setup lang="ts">
import { computed, ref, useTemplateRef, watch } from 'vue'
import { useElementSize, useEventListener } from '@vueuse/core'

/**
 * A row of text that is still working: one band of light sweeps across the whole
 * row, however many parts of different colors it holds ("Command finished:" and
 * its title, a label and its detail), as Claude and Cursor mark a step in
 * progress.
 *
 * The row shows as it is. Over it lies an inert copy in the strongest text color,
 * which a moving mask shows only where the band is, so every part keeps its own
 * color outside the band.
 *
 * The band moves at one speed on any row, so a short label does not crawl and a
 * long one does not race. A row that stops working finishes the sweep under way
 * rather than cutting it: the band goes once it has crossed the row.
 *
 * The root is a flex row: the caller gives it its gap and alignment, and the copy
 * takes the same.
 */
const props = defineProps<{ active?: boolean }>()

/**
 * The band's speed across the row, in pixels per second, and the bounds of
 * one sweep: about two seconds, as ChatGPT's and Claude's Thinking take.
 */
const SPEED_PX_S = 140
const MIN_SWEEP_MS = 1_600
const MAX_SWEEP_MS = 2_600
/**
 * The band's width, in the row's own font size: a word lit at once, with a soft edge
 * on each side, as Claude's Thinking reads.
 *
 * At 4em only a letter or two was lit and the band read as a thin line. At 8em a
 * short row such as Thinking lit whole and read as a pulse.
 */
const BAND_EM = 6

const root = useTemplateRef<HTMLElement>('root')
const band = useTemplateRef<HTMLElement>('band')
const { width } = useElementSize(root)

/** The row stopped working while a sweep was under way: the band stays until that sweep ends. */
const finishing = ref(false)
const shown = computed(() => props.active || finishing.value)
watch(() => props.active, (active) => {
  // A band that does not move, as with reduced motion or in a document
  // without the Web Animations API, has no sweep to finish.
  finishing.value = !active && (band.value?.getAnimations?.().some((animation) => animation.playState === 'running') ?? false)
})
// An infinite animation reports the end of each sweep; the one that ends after the row stopped is the last.
useEventListener(band, 'animationiteration', () => {
  finishing.value = false
})

const sweep = computed(() => {
  if (!shown.value || !root.value) {
    return undefined
  }
  const bandPx = BAND_EM * Number.parseFloat(getComputedStyle(root.value).fontSize)
  const ms = Math.round(((width.value + bandPx) / SPEED_PX_S) * 1000)
  return {
    '--shimmer-band': `${BAND_EM}em`,
    '--shimmer-ms': `${Math.min(MAX_SWEEP_MS, Math.max(MIN_SWEEP_MS, ms))}ms`,
  }
})
</script>

<template>
  <div ref="root" class="relative flex min-w-0" :style="sweep">
    <slot />
    <div v-if="shown" ref="band" class="shimmer-band" aria-hidden="true" inert>
      <slot />
    </div>
  </div>
</template>

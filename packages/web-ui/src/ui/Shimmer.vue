<script setup lang="ts">
import { computed, useTemplateRef } from 'vue'
import { useElementSize } from '@vueuse/core'

/**
 * A row of text that is still working: one band of light sweeps across the
 * whole row, however many parts of different colors it holds ("Command
 * finished:" and its title, a label and its detail), as Claude and Cursor
 * mark a step in progress. The row shows as it is; over it lies an inert
 * copy in the strongest text color, which a moving mask shows only where
 * the band is, so every part keeps its own color outside the band. The band
 * moves at one speed on any row, so a short label does not crawl and a long
 * one does not race.
 *
 * The root is a flex row: the caller gives it its gap and alignment, and the
 * copy takes the same.
 */
const props = defineProps<{ active?: boolean }>()

/**
 * The band's speed across the row, in pixels per second, and the bounds of
 * one sweep: about two seconds, as ChatGPT's and Claude's Thinking take.
 */
const SPEED_PX_S = 140
const MIN_SWEEP_MS = 1_600
const MAX_SWEEP_MS = 2_600
/** The band's width, in the row's own font size. */
const BAND_EM = 4

const root = useTemplateRef<HTMLElement>('root')
const { width } = useElementSize(root)
const sweep = computed(() => {
  if (!props.active || !root.value) {
    return undefined
  }
  const band = BAND_EM * Number.parseFloat(getComputedStyle(root.value).fontSize)
  const ms = Math.round(((width.value + band) / SPEED_PX_S) * 1000)
  return {
    '--shimmer-band': `${BAND_EM}em`,
    '--shimmer-ms': `${Math.min(MAX_SWEEP_MS, Math.max(MIN_SWEEP_MS, ms))}ms`,
  }
})
</script>

<template>
  <div ref="root" class="relative flex min-w-0" :style="sweep">
    <slot />
    <div v-if="active" class="shimmer-band" aria-hidden="true" inert>
      <slot />
    </div>
  </div>
</template>

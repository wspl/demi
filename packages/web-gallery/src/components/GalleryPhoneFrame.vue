<script setup lang="ts">
import { computed } from 'vue'

/**
 * An iPhone 15 showing a live page of the gallery at its own size, 393 × 852,
 * or the narrowest phone the app meets, an iPhone SE of the first
 * generation, 320 × 568: the page is an iframe, so its layout meets a phone's
 * width as the product's does on the phone itself. The status bar, and the
 * iPhone 15's home indicator, are drawn over the page's top and bottom edges,
 * which the page keeps clear as it keeps a phone's safe areas
 * (`/phone-frame` gives them that iPhone's values).
 */
const props = defineProps<{
  /** What the frame shows: a scene of `path`. */
  scene: string
  /** Which phone; the iPhone 15 without one. */
  device?: 'iphone-15' | 'iphone-se'
  /** The page whose scene it shows; `/phone-frame` without one. */
  path?: string
  /** The caption under the phone. */
  label: string
  /** Whether the page loads now; a page of phones loads them one at a time, as each is a whole app. */
  active?: boolean
}>()
const emit = defineEmits<{
  /** The page in the frame has loaded. */
  loaded: []
}>()

const se = computed(() => props.device === 'iphone-se')
const src = computed(() => `${props.path ?? '/phone-frame'}?scene=${encodeURIComponent(props.scene)}${se.value ? '&device=se' : ''}`)
</script>

<template>
  <figure class="flex flex-col items-center gap-3">
    <div
      class="relative border-[#0c0c0e] bg-[#0c0c0e] shadow-xl ring-1 ring-line"
      :class="se ? 'rounded-[2.4rem] border-x-[0.75rem] border-y-[4.5rem]' : 'rounded-[3.6rem] border-[0.75rem]'"
    >
      <div
        class="relative overflow-hidden bg-surface"
        :class="se ? 'h-[568px] w-[320px]' : 'h-[852px] w-[393px] rounded-[2.85rem]'"
      >
        <iframe v-if="active !== false" :src="src" :title="label" class="block h-full w-full border-0" @load="emit('loaded')" />
        <!-- The status bar: the time, the iPhone 15's Dynamic Island and the signal, Wi-Fi and battery. -->
        <div
          class="pointer-events-none absolute inset-x-0 top-0 flex items-center justify-between font-semibold text-fg-emphasis"
          :class="se ? 'h-5 px-1.5 text-[12px]' : 'h-[59px] px-8 pt-1 text-[16px]'"
        >
          <span class="w-14" :class="se ? '' : 'text-center'">9:41</span>
          <span v-if="!se" class="absolute left-1/2 top-[11px] h-[37px] w-[126px] -translate-x-1/2 rounded-full bg-black" />
          <span class="flex w-14 items-center justify-end gap-1.5">
            <span class="flex items-end gap-[2px]">
              <span class="h-1 w-[3px] rounded-sm bg-current" />
              <span class="h-1.5 w-[3px] rounded-sm bg-current" />
              <span class="h-2 w-[3px] rounded-sm bg-current" />
              <span class="h-2.5 w-[3px] rounded-sm bg-current" />
            </span>
            <span class="relative h-3 w-6 rounded-[4px] border border-current p-[1.5px] opacity-90">
              <span class="block h-full w-4/5 rounded-[2px] bg-current" />
            </span>
          </span>
        </div>
        <div v-if="!se" class="pointer-events-none absolute bottom-2 left-1/2 h-[5px] w-[134px] -translate-x-1/2 rounded-full bg-fg-emphasis" />
      </div>
    </div>
    <figcaption class="text-[12px] text-fg-subtle">{{ label }}</figcaption>
  </figure>
</template>

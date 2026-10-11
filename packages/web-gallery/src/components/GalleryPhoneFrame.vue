<script setup lang="ts">
import { computed } from 'vue'

/**
 * An iPhone 15 showing a live page of the gallery at its own size, 393 × 852:
 * the page is an iframe, so its layout meets a phone's width as the
 * product's does on the phone itself. The status bar and the home indicator
 * are drawn over the page's top and bottom edges, which the page keeps clear
 * as it keeps a phone's safe areas (`/phone-frame` gives them the iPhone's
 * values).
 */
const props = defineProps<{
  /** What the frame shows: a scene of `/phone-frame`. */
  scene: string
  /** The caption under the phone. */
  label: string
  /** Whether the page loads now; a page of phones loads them one at a time, as each is a whole app. */
  active?: boolean
}>()
const emit = defineEmits<{
  /** The page in the frame has loaded. */
  loaded: []
}>()

const src = computed(() => `/phone-frame?scene=${encodeURIComponent(props.scene)}`)
</script>

<template>
  <figure class="flex flex-col items-center gap-3">
    <div class="relative rounded-[3.6rem] border-[0.75rem] border-[#0c0c0e] bg-[#0c0c0e] shadow-xl ring-1 ring-line">
      <div class="relative h-[852px] w-[393px] overflow-hidden rounded-[2.85rem] bg-surface">
        <iframe v-if="active !== false" :src="src" :title="label" class="block h-full w-full border-0" @load="emit('loaded')" />
        <!-- The status bar: the time, the Dynamic Island and the signal, Wi-Fi and battery. -->
        <div class="pointer-events-none absolute inset-x-0 top-0 flex h-[59px] items-center justify-between px-8 pt-1 text-[16px] font-semibold text-fg-emphasis">
          <span class="w-14 text-center">9:41</span>
          <span class="absolute left-1/2 top-[11px] h-[37px] w-[126px] -translate-x-1/2 rounded-full bg-black" />
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
        <div class="pointer-events-none absolute bottom-2 left-1/2 h-[5px] w-[134px] -translate-x-1/2 rounded-full bg-fg-emphasis" />
      </div>
    </div>
    <figcaption class="text-[12px] text-fg-subtle">{{ label }}</figcaption>
  </figure>
</template>

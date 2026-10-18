<script setup lang="ts">
import { computed } from 'vue'

/**
 * A vendor's mark: a monochrome SVG (models.dev's logos, for one) painted
 * through a mask, so it takes the ink around it. A mark identifies its
 * vendor, as an account's logo does in macOS Settings, so on a tile it takes
 * the text's full ink; the initial a vendor without a mark shows is a
 * stand-in and stays muted. `row` is the bare mark at a menu row's
 * brand-mark size, 16 px, in the row's own ink (`vendorGlyph`); `sm` and
 * `md` sit on a tile.
 *
 * It is decorative: every place that draws it names the vendor beside it,
 * so it is hidden from assistive technology rather than read twice.
 */
const props = withDefaults(defineProps<{
  label: string
  src?: string | null
  size?: 'row' | 'sm' | 'md'
}>(), {
  src: null,
  size: 'md',
})

const initial = computed(() => props.label.trim().slice(0, 1).toUpperCase())
</script>

<template>
  <span
    class="inline-flex shrink-0 select-none items-center justify-center"
    :class="size === 'row' ? 'size-4' : ['rounded-md bg-overlay/8', size === 'sm' ? 'size-6' : 'size-7']"
    aria-hidden="true"
  >
    <span
      v-if="src"
      class="block bg-current"
      :class="[size === 'sm' ? 'size-3.5' : 'size-4', size === 'row' ? undefined : 'text-fg']"
      :style="{ maskImage: `url(${JSON.stringify(src)})`, maskSize: 'contain', maskRepeat: 'no-repeat', maskPosition: 'center' }"
    />
    <span
      v-else
      class="font-medium"
      :class="size === 'row' ? 'text-[10px]' : ['text-fg-muted', size === 'sm' ? 'text-[11px]' : 'text-[12px]']"
    >{{ initial }}</span>
  </span>
</template>

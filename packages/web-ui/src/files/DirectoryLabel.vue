<script setup lang="ts">
import { computed } from 'vue'
import { directoryParts } from './paths'

/**
 * A directory as a row of a list reads: its own name first, in the row's text,
 * and where it is after it, smaller and grey at the row's end, so `demi` in
 * `~/Projects` and `demi` in `~/Work` differ at a glance.
 *
 * - The home is `~` and the root `/`, with no location.
 * - A row that does not fit gives up the location before it cuts the name,
 *   from the location's left, so the folder next to the name is the last to go.
 * - The row carries the whole path as its tip.
 * - `name` stands in for the directory's own name where a place has a label of
 *   its own, such as Home.
 * - A narrow list, such as the file browser's rail, shows the location only on
 *   the rows whose names collide.
 */
const props = defineProps<{
  path: string
  /** The Host's home, which `~` writes; without it the location is a plain path. */
  home?: string | null
  name?: string
  /** Leaves the location out; a narrow list keeps it only for names that two rows share. */
  hideLocation?: boolean
}>()

const parts = computed(() => directoryParts(props.path, props.home))
</script>

<template>
  <span class="grid min-w-0 flex-1 grid-cols-[minmax(0,max-content)_minmax(0,1fr)] items-baseline">
    <span class="truncate">{{ name ?? parts.name }}</span>
    <!-- Right to left puts the cut, and its ellipsis, on the left; the isolate keeps `/` and `~` where they read. -->
    <span
      v-if="parts.location && !hideLocation"
      dir="rtl"
      class="ml-3 truncate text-[0.85em] text-fg-subtle"
    ><bdi>{{ parts.location }}</bdi></span>
  </span>
</template>

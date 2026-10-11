<script setup lang="ts">
import { ICON_PX } from '../../ui/icon-metrics'

/**
 * The lines of a row's media that are gone, under the row, each saying why
 * it was not stored (`file-previews.md` § Media a tool returned); the
 * row's pictures stand in its own line.
 */
defineProps<{ lines: readonly string[] }>()
</script>

<template>
  <div
    v-if="lines.length > 0"
    class="py-1"
    :style="{ paddingLeft: `${ICON_PX.in28 + 8}px` }"
  >
    <p
      v-for="(line, index) in lines"
      :key="index"
      class="whitespace-pre-wrap break-words font-mono text-xs leading-5 text-fg-subtle"
    >{{ line }}</p>
  </div>
</template>

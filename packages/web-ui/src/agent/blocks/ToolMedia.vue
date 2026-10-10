<script setup lang="ts">
import { ICON_PX } from '../../ui/icon-metrics'
import type { ToolMedium } from '../tool-media'
import MediumThumbnail from '../MediumThumbnail.vue'

/**
 * The images and videos a call's result carries, under the call's row or a
 * folded group's, side by side in the order of the result and wrapping onto
 * the next row, whether the call is folded or open; a medium that is gone
 * shows a line that says why where it was (`file-previews.md` § Media a tool
 * returned).
 */
defineProps<{ media: readonly ToolMedium[] }>()
</script>

<template>
  <div
    v-if="media.length > 0"
    class="flex flex-wrap items-start gap-2 py-1"
    :style="{ paddingLeft: `${ICON_PX.in28 + 8}px` }"
  >
    <template
      v-for="(medium, index) in media"
      :key="index"
    >
      <p
        v-if="medium.kind === 'gone'"
        class="w-full whitespace-pre-wrap break-words font-mono text-xs leading-5 text-fg-subtle"
      >{{ medium.text }}</p>
      <MediumThumbnail
        v-else
        :kind="medium.kind"
        :source="medium.source"
        :name="medium.name"
      />
    </template>
  </div>
</template>

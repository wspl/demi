<script setup lang="ts">
import { computed } from 'vue'
import type { ToolResultContentBlock } from '@demicodes/protocol'
import { ICON_PX } from '../../ui/icon-metrics'
import { toolMedia } from '../tool-media'
import ToolMediumPreview from './ToolMediumPreview.vue'

/**
 * The images and videos a call's result carries, under the call's row and
 * in the order of the result, whether the call is folded or open; a medium
 * that is gone shows a line that says why where it was
 * (`file-previews.md` § Media a tool returned).
 */
const props = defineProps<{
  output: readonly ToolResultContentBlock[]
  /** The call's title, which names each medium. */
  title: string
}>()

const media = computed(() => toolMedia(props.output))
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
      <ToolMediumPreview
        v-else
        :kind="medium.kind"
        :source="medium.source"
        :name="title"
      />
    </template>
  </div>
</template>

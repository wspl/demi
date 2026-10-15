<script setup lang="ts">
import { computed } from 'vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from './FunctionalBlock.vue'
import GoneMediaLines from './GoneMediaLines.vue'
import ToolCallFace from './ToolCallFace.vue'
import { toolRowIcon } from './tool-row-icon'
import type { ToolCallBlock } from '../block-types'
import { splitMedia, toolMedia } from '../tool-media'

const props = defineProps<{
  block: ToolCallBlock
}>()
const isOpen = defineModel<boolean>('open', { default: false })

// The result's own text; its pictures show in the row's line, and a medium that is gone under it.
const texts = computed(() => props.block.output.flatMap((part) => part.type === 'text' ? [part.text] : []))
const media = computed(() => splitMedia(toolMedia(props.block.output, props.block.toolName)))
</script>

<template>
  <FunctionalBlock
    v-model:open="isOpen"
    :loading="block.status === 'executing'"
    :pictures="media.pictures"
  >
    <ToolCallFace :block="block" />
    <template
      v-if="texts.length > 0"
      #body
    >
      <div class="space-y-2 py-2">
        <pre
          v-for="(text, index) in texts"
          :key="index"
          class="whitespace-pre-wrap break-words text-chrome"
          >{{ text }}</pre
        >
      </div>
    </template>
    <template #icon>
      <component :is="toolRowIcon(block.toolName)" :size="ICON_PX.in28" />
    </template>
  </FunctionalBlock>
  <GoneMediaLines :lines="media.gone" />
</template>

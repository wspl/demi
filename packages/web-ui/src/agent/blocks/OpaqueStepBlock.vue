<script setup lang="ts">
import { computed } from 'vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from './FunctionalBlock.vue'
import OpaqueOtherFace from './OpaqueOtherFace.vue'
import OpaqueSearchFace from './OpaqueSearchFace.vue'
import OpaqueSearchSources from './OpaqueSearchSources.vue'
import { opaqueIcon } from './tool-row-icon'
import type { OpaqueBlock } from '../block-types'

/**
 * A vendor's step as a row of a run, drawn by its view: each kind of view has
 * a line of its own. Redacted reasoning never reaches here; the transcript
 * shows it as thinking (`visible-blocks.ts`).
 */
const props = defineProps<{
  block: OpaqueBlock
}>()
const sources = computed(() => props.block.view.kind === 'search' ? props.block.view.sources : [])
const isOpen = defineModel<boolean>('open', { default: false })
</script>

<template>
  <FunctionalBlock v-model:open="isOpen" :loading="block.status === 'running'">
    <OpaqueSearchFace v-if="block.view.kind === 'search'" :block="block" :view="block.view" />
    <OpaqueOtherFace v-else-if="block.view.kind === 'other'" :block="block" :view="block.view" />
    <template v-if="sources.length > 0" #body>
      <OpaqueSearchSources :sources="sources" />
    </template>
    <template #icon>
      <component :is="opaqueIcon(block.view)" :size="ICON_PX.in28" />
    </template>
  </FunctionalBlock>
</template>

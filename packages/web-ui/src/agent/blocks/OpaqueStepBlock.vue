<script setup lang="ts">
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from './FunctionalBlock.vue'
import OpaqueOtherFace from './OpaqueOtherFace.vue'
import { OPAQUE_ICON } from './tool-row-icon'
import type { OpaqueBlock } from '../block-types'

/**
 * A vendor's step as a row of a run, drawn by its view: each kind of view has
 * a line of its own. Redacted reasoning never reaches here; the transcript
 * shows it as thinking (`visible-blocks.ts`).
 */
defineProps<{
  block: OpaqueBlock
}>()
const isOpen = defineModel<boolean>('open', { default: false })
</script>

<template>
  <FunctionalBlock v-model:open="isOpen" :loading="block.status === 'running'">
    <OpaqueOtherFace v-if="block.view.kind === 'other'" :block="block" :view="block.view" />
    <template #icon>
      <component :is="OPAQUE_ICON" :size="ICON_PX.in28" />
    </template>
  </FunctionalBlock>
</template>

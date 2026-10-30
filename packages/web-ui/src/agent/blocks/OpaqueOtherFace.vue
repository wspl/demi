<script setup lang="ts">
import RunEndTag from './RunEndTag.vue'
import type { OpaqueBlock } from '../block-types'
import { t } from '../locales/text'

/**
 * The line of a vendor's step of a kind Demi does not know: the step's name
 * and its vendor, with no tense, since it reads the same while the step runs
 * and once it ended; marked Failed when it failed.
 */
defineProps<{
  block: OpaqueBlock
  view: Extract<OpaqueBlock['view'], { kind: 'other' }>
}>()
</script>

<template>
  <span class="min-w-0 truncate">{{ t('transcript.opaque.other', { vendor: view.vendor, name: view.name }) }}</span>
  <RunEndTag v-if="block.status === 'failed'" kind="failed" />
</template>

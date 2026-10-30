<script setup lang="ts">
import { computed } from 'vue'
import RunEndTag from './RunEndTag.vue'
import type { OpaqueBlock } from '../block-types'
import { searchSubject, searchTitle } from '../opaque-steps'

/**
 * The line of a vendor's search: what it did, Web Search or X Search, and
 * its words, the query or the page; marked Failed when it failed.
 */
const props = defineProps<{
  block: OpaqueBlock
  view: Extract<OpaqueBlock['view'], { kind: 'search' }>
}>()

const subject = computed(() => searchSubject(props.view.action))
</script>

<template>
  <span class="shrink-0">{{ searchTitle(view.action) }}</span>
  <span v-if="subject" class="min-w-0 truncate text-fg-muted group-hover:text-fg-body">{{ subject }}</span>
  <RunEndTag v-if="block.status === 'failed'" kind="failed" />
</template>

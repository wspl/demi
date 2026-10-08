<script setup lang="ts">
import { computed } from 'vue'
import { FileDiff } from '@lucide/vue'
import type { TranscriptRequest } from '../../files/request-changes'
import { ICON_PX } from '../../ui/icon-metrics'

/**
 * The button at the end of a request's reply (`edit-tracking.md` § What the
 * conversation shows): how many files the request's calls changed. It grows
 * as later calls of the request end, and gives no line counts, which only
 * the Change view's All Changes tells right. A pill like the file pills
 * under a call: a control while a page opens the request's changes, plain
 * text otherwise.
 */
const props = defineProps<{
  request: TranscriptRequest
  selectable: boolean
}>()
const emit = defineEmits<{ open: [] }>()

const label = computed(() => props.request.files.length === 1 ? '1 File Changed' : `${props.request.files.length} Files Changed`)
</script>

<template>
  <component
    :is="selectable ? 'button' : 'span'"
    :type="selectable ? 'button' : undefined"
    class="inline-flex h-[22px] select-none items-center gap-1.5 rounded-full bg-btn pl-1.5 pr-2 text-xs leading-4 text-fg-body shadow-[var(--shadow-btn)]"
    :class="selectable ? 'btn' : ''"
    @click="selectable && emit('open')"
  >
    <FileDiff :size="ICON_PX.in28" class="shrink-0 text-fg-muted" aria-hidden="true" />
    {{ label }}
  </component>
</template>

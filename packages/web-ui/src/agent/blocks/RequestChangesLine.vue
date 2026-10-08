<script setup lang="ts">
import { computed } from 'vue'
import { FileDiff } from '@lucide/vue'
import LineCounts from '../../files/LineCounts.vue'
import type { TranscriptRequest } from '../../files/request-changes'
import { ICON_PX } from '../../ui/icon-metrics'

/**
 * The line at the end of a request's reply (`edit-tracking.md` § What the
 * conversation shows): how many files the request's calls changed, and the
 * lines they added and removed. It grows as later calls of the request end.
 * A pill like the file pills under a call: a control that opens the
 * request's changes while a page opens them, plain text otherwise.
 */
const props = defineProps<{
  request: TranscriptRequest
  selectable: boolean
}>()
const emit = defineEmits<{ open: [] }>()

const label = computed(() => props.request.files.length === 1 ? '1 File Changed' : `${props.request.files.length} Files Changed`)
const totals = computed(() => props.request.files.reduce(
  (sum, file) => ({ added: sum.added + file.added, removed: sum.removed + file.removed }),
  { added: 0, removed: 0 },
))
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)] py-1">
    <component
      :is="selectable ? 'button' : 'span'"
      :type="selectable ? 'button' : undefined"
      class="inline-flex h-[22px] select-none items-center gap-1.5 rounded-full bg-btn pl-1.5 pr-2 text-xs leading-4 text-fg-body shadow-[var(--shadow-btn)]"
      :class="selectable ? 'btn' : ''"
      @click="selectable && emit('open')"
    >
      <FileDiff :size="ICON_PX.in28" class="shrink-0 text-fg-muted" aria-hidden="true" />
      <!-- Label and counts use different fonts and sizes: align them on the baseline, as the file pills do. -->
      <span class="inline-flex items-baseline gap-1.5">
        <span>{{ label }}</span>
        <LineCounts :added="totals.added" :removed="totals.removed" />
      </span>
    </component>
  </div>
</template>

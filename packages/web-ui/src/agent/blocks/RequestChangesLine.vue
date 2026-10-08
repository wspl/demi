<script setup lang="ts">
import { computed, ref } from 'vue'
import { useElementVisibility } from '@vueuse/core'
import { FileDiff } from '@lucide/vue'
import LineCounts from '../../files/LineCounts.vue'
import type { TranscriptRequest } from '../../files/request-changes'
import { ICON_PX } from '../../ui/icon-metrics'
import { useFileLineCounts } from '../useFileLineCounts'

/**
 * The button at the end of a request's reply (`edit-tracking.md` § What the
 * conversation shows): how many files the request's calls changed and the
 * lines added and removed across them, each file's All Changes, once the
 * request has ended (`requestLineIds`). Once the button comes into view it reads
 * each file's two ends; until they are counted, or when a file's ends were
 * not kept, it names the files alone, and the counts then appear at its end
 * without moving anything. A pill like the file pills under a call: a
 * control while a page opens the request's changes, plain text otherwise.
 */
const props = defineProps<{
  request: TranscriptRequest
  selectable: boolean
}>()
const emit = defineEmits<{ open: [] }>()

const label = computed(() => props.request.files.length === 1 ? '1 File Changed' : `${props.request.files.length} Files Changed`)

const el = ref<HTMLElement | null>(null)
const visible = useElementVisibility(el)
const byFile = useFileLineCounts(() => props.request.files, () => visible.value)
/** The sum over the files, once each is counted; none while a file's ends are missing. */
const counts = computed(() => {
  let added = 0
  let removed = 0
  for (const count of byFile.value.values()) {
    if (!count)
      return null
    added += count.added
    removed += count.removed
  }
  return { added, removed }
})
</script>

<template>
  <component
    :is="selectable ? 'button' : 'span'"
    ref="el"
    :type="selectable ? 'button' : undefined"
    class="inline-flex h-[22px] select-none items-center gap-1.5 rounded-full bg-btn pl-1.5 pr-2 text-xs leading-4 text-fg-body shadow-[var(--shadow-btn)]"
    :class="selectable ? 'btn' : ''"
    @click="selectable && emit('open')"
  >
    <FileDiff :size="ICON_PX.in28" class="shrink-0 text-fg-muted" aria-hidden="true" />
    <!-- Label and counts use different fonts and sizes: align them on the baseline, as the file pills do. -->
    <span class="inline-flex items-baseline gap-1.5">
      <span>{{ label }}</span>
      <LineCounts v-if="counts" :added="counts.added" :removed="counts.removed" zeros />
    </span>
  </component>
</template>

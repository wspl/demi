<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useElementVisibility } from '@vueuse/core'
import { FileDiff } from '@lucide/vue'
import LineCounts from '../../files/LineCounts.vue'
import { requestEndsKey, requestLineCounts, type TranscriptRequest } from '../../files/request-changes'
import { ICON_PX } from '../../ui/icon-metrics'
import { useEditReads } from '../edit-selection'

/**
 * The button at the end of a request's reply (`edit-tracking.md` § What the
 * conversation shows): how many files the request's calls changed and the
 * lines added and removed across them, each file's All Changes. It grows as
 * later calls of the request end. Once the button comes into view it reads
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
const read = useEditReads()
/** The files' ends the counts were taken from, so counts of ends a later call replaced never show. */
const ends = computed(() => requestEndsKey(props.request))
const counted = ref<{ ends: string; added: number; removed: number } | null>(null)
const counts = computed(() => counted.value?.ends === ends.value ? counted.value : null)
let controller: AbortController | null = null

watch([visible, ends], async ([shown, key]) => {
  const reader = read()
  if (!shown || !reader || counted.value?.ends === key) {
    return
  }
  controller?.abort()
  const current = new AbortController()
  controller = current
  try {
    const result = await requestLineCounts(props.request, reader, current.signal)
    if (!current.signal.aborted && result) {
      counted.value = { ends: key, ...result }
    }
  } catch {
    // Safe to ignore: an abort is the next count taking over, and a read that
    // failed leaves the button naming the files alone, as the design has it
    // for ends it cannot count; the Change view reports a failed read itself.
  }
}, { immediate: true })

onBeforeUnmount(() => controller?.abort())
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

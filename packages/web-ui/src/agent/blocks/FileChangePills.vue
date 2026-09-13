<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import FileIcon from '@demicodes/web-ui/files/FileIcon.vue'
import CornerDot from '../../ui/CornerDot.vue'
import type { ChangeFile } from '../../files/changes'

/**
 * The files a tool call changed, as pills that wrap: icon, name, and the
 * line counts. A new file carries a green dot on its icon; a deleted file is
 * struck through; a renamed file names its old path in the tooltip.
 * Collapsed, the pills stop after `maxRows` rows and a "+N files" pill at
 * the end of the last row shows the rest; "Show less" folds them again.
 */
const props = withDefaults(
  defineProps<{
    files: ChangeFile[]
    maxRows?: number
  }>(),
  { maxRows: 3 },
)

const emit = defineEmits<{ select: [path: string] }>()

const container = ref<HTMLElement>()
const pillEls = ref<HTMLElement[]>([])
const moreEl = ref<HTMLElement>()
const expanded = ref(false)
/** How many pills fit within `maxRows` with the "+N files" pill beside them. */
const fitCount = ref(props.files.length)

const visibleCount = computed(() => expanded.value ? props.files.length : fitCount.value)
const hidden = computed(() => props.files.length - visibleCount.value)

function baseName(path: string): string {
  const normalized = path.replaceAll('\\', '/')
  return normalized.slice(normalized.lastIndexOf('/') + 1)
}

function rowOf(el: HTMLElement, top: number, rowHeight: number): number {
  return Math.round((el.offsetTop - top) / rowHeight)
}

// Lay every pill out, then cut at the first pill on a row past the limit and
// keep cutting until the "+N files" pill lands on the last allowed row too.
async function measure(): Promise<void> {
  if (expanded.value || !container.value)
    return
  fitCount.value = props.files.length
  await nextTick()
  const first = pillEls.value[0]
  if (!first || !container.value)
    return
  const top = first.offsetTop
  const rowHeight = first.offsetHeight + parseFloat(getComputedStyle(container.value).rowGap)
  const overflowIndex = pillEls.value.findIndex((el) => rowOf(el, top, rowHeight) >= props.maxRows)
  if (overflowIndex < 0)
    return
  let count = overflowIndex
  while (count > 0) {
    fitCount.value = count
    await nextTick()
    if (!moreEl.value || rowOf(moreEl.value, top, rowHeight) < props.maxRows)
      return
    count -= 1
  }
  fitCount.value = count
}

let observer: ResizeObserver | undefined

onMounted(() => {
  if (container.value) {
    observer = new ResizeObserver(() => { void measure() })
    observer.observe(container.value)
  }
  void measure()
})

onBeforeUnmount(() => {
  observer?.disconnect()
})

watch([() => props.files, expanded], () => { void measure() })
</script>

<template>
  <div
    ref="container"
    class="flex min-w-0 flex-wrap items-center gap-1"
  >
    <button
      type="button"
      v-for="(file, index) in files"
      v-show="index < visibleCount"
      :key="file.path"
      ref="pillEls"
      class="btn inline-flex h-[22px] max-w-64 select-none items-center gap-1.5 rounded-full bg-[var(--btn-bg)] pl-1.5 pr-2 text-xs leading-4 text-fg-body shadow-[var(--shadow-btn)]"
      @click="emit('select', file.path)"
      :title="file.from ? `${file.from} → ${file.path}` : file.path"
    >
      <span class="relative inline-flex shrink-0">
        <FileIcon :name="baseName(file.path)" :is-directory="false" :size="14" />
        <CornerDot v-if="file.kind === 'added'" tone="success" size="xs" ring="button" />
      </span>
      <!-- Name and counts use different fonts and sizes: align them on the baseline, not the box. -->
      <span class="inline-flex min-w-0 items-baseline gap-1.5">
        <span
          class="min-w-0 truncate"
          :class="file.kind === 'deleted' ? 'line-through text-fg-muted' : ''"
        >{{ baseName(file.path) }}</span>
        <span class="inline-flex shrink-0 gap-1 font-mono text-[11px] tabular-nums">
          <span v-if="file.added > 0" class="text-on-success">+{{ file.added }}</span>
          <span v-if="file.removed > 0" class="text-on-danger">−{{ file.removed }}</span>
        </span>
      </span>
    </button>
    <button
      v-if="hidden > 0 || expanded"
      ref="moreEl"
      type="button"
      class="btn inline-flex h-[22px] cursor-default select-none items-center rounded-full px-2 text-xs leading-4 text-fg-muted transition-colors duration-200 ease-out hover:text-fg-body"
      @click="expanded = !expanded"
    >{{ expanded ? 'Show less' : `+${hidden} files` }}</button>
  </div>
</template>

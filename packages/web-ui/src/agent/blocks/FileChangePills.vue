<script setup lang="ts">
import LineCounts from '../../files/LineCounts.vue'
import { computed, ref } from 'vue'
import { baseName } from '@demicodes/utils'
import FileIcon from '@demicodes/web-ui/files/FileIcon.vue'
import CornerPlus from '../../ui/CornerPlus.vue'
import { useFitMore } from '../../ui/fit-more'
import { textWidth } from '../../ui/text-width'
import type { ChangeFile } from '../../files/changes'

/**
 * The files a tool call changed, as pills that wrap: icon, name, and the
 * line counts. A new file carries a green plus at its icon's lower right
 * corner, as a file added in an editor's tree does; a deleted file is
 * struck through; a renamed file names its old path in the tooltip.
 * Collapsed, the pills stop after `maxRows` rows and a "+N files" pill at
 * the end of the last row shows the rest; "Show Less" folds them again.
 */
const props = withDefaults(
  defineProps<{
    files: ChangeFile[]
    maxRows?: number
    /**
     * Which pills open their file's edit, all or each by its path; any
     * other only names its file.
     */
    selectable?: boolean | ((path: string) => boolean)
  }>(),
  { maxRows: 3, selectable: true },
)

const emit = defineEmits<{ select: [path: string] }>()

function opens(path: string): boolean {
  return typeof props.selectable === 'function' ? props.selectable(path) : props.selectable
}

const container = ref<HTMLElement>()
const pillEls = ref<HTMLElement[]>([])
const expanded = ref(false)

/** The "+N Files" button's width beside its text: its side paddings (`px-2`). */
const MORE_PADDING_PX = 16

/** How many pills fit within `maxRows` rows with the "+N Files" button after them. */
const fitCount = useFitMore({
  items: () => props.files,
  box: () => container.value,
  enabled: () => !expanded.value,
  measure: () => {
    const box = container.value
    const first = pillEls.value[0]
    if (!box || !first)
      return null
    const style = getComputedStyle(box)
    const font = getComputedStyle(first).font
    const room = box.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)
    return {
      room,
      // A pill wider than the line shrinks to it, truncating its name.
      widths: pillEls.value.map((pill) => Math.min(pill.offsetWidth, room)),
      gap: parseFloat(style.columnGap),
      rows: props.maxRows,
      countWidth: (rest) => Math.ceil(textWidth(`+${rest} Files`, font)) + MORE_PADDING_PX,
    }
  },
})

const visibleCount = computed(() => expanded.value ? props.files.length : fitCount.value)
const hidden = computed(() => props.files.length - visibleCount.value)
</script>

<template>
  <div
    ref="container"
    class="flex min-w-0 flex-wrap items-center gap-1"
  >
    <component
      :is="opens(file.path) ? 'button' : 'span'"
      :type="opens(file.path) ? 'button' : undefined"
      v-for="(file, index) in files"
      v-show="index < visibleCount"
      :key="file.path"
      ref="pillEls"
      class="file-pill group/pill inline-flex h-[22px] max-w-64 select-none items-center gap-1.5 rounded-full pl-1.5 pr-2 text-xs leading-4 text-fg-muted transition-colors duration-200 ease-out"
      :class="opens(file.path) ? 'btn [--shadow-btn:var(--shadow-pill)] hover:text-fg-body' : 'bg-btn shadow-[var(--shadow-pill)]'"
      @click="opens(file.path) && emit('select', file.path)"
      :title="file.from ? `${file.from} → ${file.path}` : file.path"
    >
      <!-- The pills stand under the row they belong to, quieter than it: the
        icon a step back too, never so faint it reads as unavailable. -->
      <span class="relative inline-flex shrink-0 opacity-80 transition-opacity duration-200 ease-out group-hover/pill:opacity-100">
        <FileIcon :name="baseName(file.path)" :is-directory="false" :size="14" />
        <CornerPlus v-if="file.kind === 'added'" label="New file" />
      </span>
      <!-- Name and counts use different fonts and sizes: align them on the baseline, not the box. -->
      <span class="inline-flex min-w-0 items-baseline gap-1.5">
        <span
          class="min-w-0 truncate"
          :class="file.kind === 'deleted' ? 'deleted-name' : ''"
        >{{ baseName(file.path) }}</span>
        <LineCounts :added="file.added" :removed="file.removed" />
      </span>
    </component>
    <button
      v-if="hidden > 0 || expanded"
      type="button"
      class="file-pill btn inline-flex h-[22px] cursor-default select-none items-center rounded-full px-2 text-xs leading-4 text-fg-muted transition-colors duration-200 ease-out hover:text-fg-body"
      @click="expanded = !expanded"
    >{{ expanded ? 'Show Less' : `+${hidden} Files` }}</button>
  </div>
</template>

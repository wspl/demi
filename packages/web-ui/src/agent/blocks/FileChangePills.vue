<script setup lang="ts">
import { t } from '../locales/text'
import LineCounts from '../../files/LineCounts.vue'
import { computed, ref } from 'vue'
import { baseName } from '@demicodes/utils'
import FileIcon from '@demicodes/web-ui/files/FileIcon.vue'
import CornerPlus from '../../ui/CornerPlus.vue'
import { useFitMore } from '../../ui/fit-more'
import { textWidth } from '../../ui/text-width'
import Tooltip from '../../ui/Tooltip.vue'
import { useIconPx } from '../../ui/icon-metrics'
import type { ChangeFile } from '../../files/changes'

/**
 * The files a tool call changed, as pills that wrap: icon, name, and the
 * line counts. A new file carries a green plus at its icon's lower right
 * corner, as a file added in an editor's tree does; a deleted file is
 * struck through; a renamed file names its old path in the tooltip.
 * The pills take at most two lines; the files that do not fit fold into a
 * last pill, "+N Files", which asks for the call's whole list (`openRest`)
 * where a pill asks for its file.
 */
const props = withDefaults(
  defineProps<{
    files: ChangeFile[]
    /**
     * Which pills open their file's edit, all or each by its path; any
     * other only names its file.
     */
    selectable?: boolean | ((path: string) => boolean)
    /**
     * Whether the "+N Files" pill opens the files it names, by their paths;
     * without it, it only counts them.
     */
    restSelectable?: (paths: readonly string[]) => boolean
  }>(),
  { selectable: true, restSelectable: undefined },
)

const emit = defineEmits<{ select: [path: string]; openRest: [paths: string[]] }>()
const icons = useIconPx()

/** The most lines the pills take. */
const MAX_ROWS = 2

function opens(path: string): boolean {
  return typeof props.selectable === 'function' ? props.selectable(path) : props.selectable
}

const container = ref<HTMLElement>()
/** The pills in the order of `files`, which `measure` lays out and each tip points at. */
const pillEls = ref<HTMLElement[]>([])

/** What the last pill says for the files that do not fit, "+3 Files". */
function moreLabel(count: number): string {
  return t('transcript.moreFiles', { count }, count)
}

/** The "+N Files" button's width beside its text: its side paddings (`px-2`). */
const MORE_PADDING_PX = 16

/** How many pills fit within `maxRows` rows with the "+N Files" button after them. */
const fitCount = useFitMore({
  items: () => props.files,
  box: () => container.value,
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
      rows: MAX_ROWS,
      countWidth: (rest) => Math.ceil(textWidth(moreLabel(rest), font)) + MORE_PADDING_PX,
    }
  },
})

const hiddenPaths = computed(() => props.files.slice(fitCount.value).map((file) => file.path))
const opensRest = computed(() => props.restSelectable?.(hiddenPaths.value) ?? false)
</script>

<template>
  <div
    ref="container"
    class="flex min-w-0 flex-wrap items-center gap-1 phone:gap-2"
  >
    <template v-for="(file, index) in files" :key="file.path">
      <component
        :is="opens(file.path) ? 'button' : 'span'"
        :type="opens(file.path) ? 'button' : undefined"
        v-show="index < fitCount"
        ref="pillEls"
        class="file-pill group/pill inline-flex h-pill max-w-64 select-none items-center gap-1.5 rounded-full pl-1.5 pr-2 text-pill leading-4 phone:pl-2 phone:pr-2.5 text-fg-muted transition-colors duration-200 ease-out"
        :class="opens(file.path) ? 'btn [--shadow-btn:var(--shadow-pill)] hover:text-fg-body' : 'bg-btn shadow-[var(--shadow-pill)]'"
        @click="opens(file.path) && emit('select', file.path)"
      >
        <!-- The pills stand under the row they belong to, quieter than it: the
          icon a step back too, never so faint it reads as unavailable. -->
        <span class="relative inline-flex shrink-0 opacity-80 transition-opacity duration-200 ease-out group-hover/pill:opacity-100">
          <FileIcon :name="baseName(file.path)" :is-directory="false" :size="icons.in20" />
          <CornerPlus v-if="file.kind === 'added'" :label="t('transcript.newFile')" />
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
      <!-- The whole path, and where a renamed file came from. -->
      <Tooltip :trigger="pillEls[index]" :content="file.from ? `${file.from} → ${file.path}` : file.path" />
    </template>
    <component
      :is="opensRest ? 'button' : 'span'"
      :type="opensRest ? 'button' : undefined"
      v-if="hiddenPaths.length > 0"
      class="file-pill inline-flex h-pill select-none items-center rounded-full px-2 text-pill leading-4 text-fg-muted transition-colors duration-200 ease-out"
      :class="opensRest ? 'btn [--shadow-btn:var(--shadow-pill)] hover:text-fg-body' : 'bg-btn shadow-[var(--shadow-pill)]'"
      @click="opensRest && emit('openRest', hiddenPaths)"
    >{{ moreLabel(hiddenPaths.length) }}</component>
  </div>
</template>

<script setup lang="ts">
import { t } from '../locales/text'
import LineCounts from '../../files/LineCounts.vue'
import { computed, ref, shallowReactive, type ComponentPublicInstance } from 'vue'
import { baseName } from '@demicodes/utils'
import FileIcon from '@demicodes/web-ui/files/FileIcon.vue'
import CornerPlus from '../../ui/CornerPlus.vue'
import { useFitMore } from '../../ui/fit-more'
import { textWidth } from '../../ui/text-width'
import Tooltip from '../../ui/Tooltip.vue'
import { useIconPx } from '../../ui/icon-metrics'
import type { ChangeFile } from '../../files/changes'

/**
 * The files a tool call changed, as pills that wrap: icon, name, and the line
 * counts.
 *
 * - A new file carries a green plus at its icon's lower right corner, as a file
 *   added in an editor's tree does.
 * - A deleted file is struck through.
 * - A renamed file names its old path in the tooltip.
 *
 * The pills take one line. The files that do not fit fold into a last pill,
 * "+N Files", which shows them in place on the lines below and turns into Show
 * Less, which folds them back. The newest file always shows, its name cut to
 * leave the count its room.
 */
const props = withDefaults(
  defineProps<{
    files: ChangeFile[]
    /**
     * Which pills open their file's edit, all or each by its path; any
     * other only names its file.
     */
    selectable?: boolean | ((path: string) => boolean)
  }>(),
  { selectable: true },
)

const emit = defineEmits<{ select: [path: string] }>()
const icons = useIconPx()

function opens(path: string): boolean {
  return typeof props.selectable === 'function' ? props.selectable(path) : props.selectable
}

const container = ref<HTMLElement>()
/**
 * Each file's pill by its path, which `measure` lays out and each tip points
 * at: by path, since a list in a new order, as newest first makes it, moves
 * the pills, and an array of template refs keeps the order they mounted in.
 */
const pillEls = shallowReactive(new Map<string, HTMLElement>())
type PillRef = (element: Element | ComponentPublicInstance | null) => void
/** One function for each path, so a render passes the same ref and Vue never unsets it in between. */
const pillRefs = new Map<string, PillRef>()
function pillRef(path: string): PillRef {
  let set = pillRefs.get(path)
  if (!set) {
    set = (element) => {
      if (element instanceof HTMLElement) {
        if (pillEls.get(path) !== element)
          pillEls.set(path, element)
      } else {
        pillEls.delete(path)
        pillRefs.delete(path)
      }
    }
    pillRefs.set(path, set)
  }
  return set
}

/** What the last pill says for the files that do not fit, "+3 Files". */
function moreLabel(count: number): string {
  return t('transcript.moreFiles', { count }, count)
}

/**
 * The "+N Files" button's width beside its text: its side paddings (`px-2`).
 */
const MORE_PADDING_PX = 16

/**
 * The width the first pill may take beside the "+N Files" button for the
 * rest, as the last measure found it.
 */
const firstRoom = ref(0)

/** How many pills fit on the line with the "+N Files" button after them. */
const fitCount = useFitMore({
  items: () => props.files,
  box: () => container.value,
  measure: () => {
    const box = container.value
    const pills = props.files.map((file) => pillEls.get(file.path))
    const first = pills[0]
    if (!box || !first || pills.some((pill) => !pill))
      return null
    const style = getComputedStyle(box)
    const font = getComputedStyle(first).font
    const room = box.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)
    const gap = parseFloat(style.columnGap)
    const countWidth = (rest: number) => Math.ceil(textWidth(moreLabel(rest), font)) + MORE_PADDING_PX
    firstRoom.value = room - gap - countWidth(props.files.length - 1)
    return {
      room,
      // A pill wider than the line shrinks to it, truncating its name.
      widths: pills.map((pill) => Math.min(pill!.offsetWidth, room)),
      gap,
      countWidth,
    }
  },
})

/**
 * Whether the first pill alone does not fit beside the button, and shows with
 * its name cut to the room the button leaves.
 */
const squeezed = computed(() => fitCount.value === 0 && props.files.length > 1 && firstRoom.value > 0)
/** How many pills show while folded. */
const shownCount = computed(() => (squeezed.value ? 1 : fitCount.value))
/** How many files the line leaves out. */
const hidden = computed(() => props.files.length - shownCount.value)
/** "+N Files" put the files it counted in place. */
const expanded = ref(false)
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
        v-show="expanded || index < shownCount"
        :ref="pillRef(file.path)"
        :style="squeezed && !expanded && index === 0 ? { maxWidth: `${firstRoom}px` } : undefined"
        class="file-pill group/pill inline-flex h-pill max-w-64 select-none items-center gap-1.5 rounded-full pl-1.5 pr-2 text-pill leading-4 phone:pl-2 phone:pr-2.5 text-fg-muted transition-colors duration-200 ease-out"
        :class="opens(file.path) ? 'btn [--shadow-btn:var(--shadow-pill)] hover:text-fg-body' : 'bg-btn shadow-[var(--shadow-pill)]'"
        @click="opens(file.path) && emit('select', file.path)"
      >
        <!--
          The pills stand under the row they belong to, quieter than it: the
          icon a step back too, never so faint it reads as unavailable.
        -->
        <span class="relative inline-flex shrink-0 opacity-80 transition-opacity duration-200 ease-out group-hover/pill:opacity-100">
          <FileIcon :name="baseName(file.path)" :is-directory="false" :size="icons.in20" />
          <CornerPlus v-if="file.kind === 'added'" :label="t('transcript.newFile')" />
        </span>
        <!--
          Name and counts use different fonts and sizes: align them on the
          baseline, not the box.
        -->
        <span class="inline-flex min-w-0 items-baseline gap-1.5">
          <span
            class="min-w-0 truncate"
            :class="file.kind === 'deleted' ? 'deleted-name' : ''"
          >{{ baseName(file.path) }}</span>
          <LineCounts :added="file.added" :removed="file.removed" />
        </span>
      </component>
      <!-- The whole path, and where a renamed file came from. -->
      <Tooltip :trigger="pillEls.get(file.path)" :content="file.from ? `${file.from} → ${file.path}` : file.path" />
    </template>
    <button
      v-if="hidden > 0"
      type="button"
      :aria-expanded="expanded"
      class="file-pill btn inline-flex h-pill select-none items-center rounded-full px-2 text-pill leading-4 text-fg-muted transition-colors duration-200 ease-out [--shadow-btn:var(--shadow-pill)] hover:text-fg-body"
      @click="expanded = !expanded"
    >{{ expanded ? t('transcript.fewerFiles') : moreLabel(hidden) }}</button>
  </div>
</template>

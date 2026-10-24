<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'
import { Search, SearchX } from '@lucide/vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import HighlightText from '../ui/HighlightText.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import TextInput from '../ui/TextInput.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { usePhone } from '../ui/phone'
import { errorMessage } from '../infra/errors'
import { formatCount } from '../ui/numbers'
import { SEARCH_PAUSE_MS } from '../search/search'
import { stepHighlight } from '../composables/list-highlight'
import FileIcon from './FileIcon.vue'
import { TextSearch, type FoundFile } from './file-search'
import { joinPath } from './paths'
import { t } from './locales/text'
import type { FileBrowserSource, FileSearchRange } from './types'

/**
 * Search in Files: a field that floats over Files and finds the files under
 * `root` on the source's Host whose name or text holds what it holds, letter
 * case aside, as the conversations' search finds conversations
 * (`crates/runner-host/src/search.rs`).
 *
 * - Once typing pauses, the files found list under the field as the Host
 *   finds them: those whose name holds the text first, the text marked in the
 *   name, then those whose text holds it, with the first line that does under
 *   the name, marked there. Each kind is in the tree's order.
 * - ↑ and ↓ move the selection, Return or a click opens the selected file, at
 *   that line for a line, and Escape or a click outside closes the field.
 * - The field is as tall as its list, up to a limit past which the list
 *   scrolls, and hangs from the top of Files, as in VS Code's Quick Open.
 * - Only the rows in view are drawn: a search lists up to five thousand files.
 * - Closing stops the search that runs.
 */
const props = defineProps<{
  source: FileBrowserSource
  /** The directory searched, by absolute path. */
  root: string
}>()

const emit = defineEmits<{
  close: []
  /** A file found, by absolute path, and the line to open it at for one found by a line. */
  open: [path: string, line: number | null]
}>()

const text = ref('')
const search = new TextSearch()
const files = search.files
const state = search.state
/**
 * The selected file, by its path, so a file found later and listed above it
 * does not move the selection to another; the first row when none is.
 */
const selectedPath = ref<string | null>(null)
const listId = useId()

let pause: ReturnType<typeof setTimeout> | undefined
watch(text, (now) => {
  clearTimeout(pause)
  if (now.trim() === '') {
    search.clear()
    return
  }
  pause = setTimeout(() => void search.run(props.source, props.root, now), SEARCH_PAUSE_MS)
})

// Another search's list starts at its first row.
watch(search.foundFor, () => {
  selectedPath.value = null
})

// What was found belongs to the source and the root it was found in.
watch([() => props.source, () => props.root], () => {
  clearTimeout(pause)
  search.clear()
  text.value = ''
})

const card = ref<HTMLElement | null>(null)
const field = ref<InstanceType<typeof TextInput> | null>(null)
/** A press anywhere but on the field and its list closes it, as a menu closes. */
function pressedOutside(event: PointerEvent): void {
  if (!(event.target instanceof Node) || !card.value?.contains(event.target))
    emit('close')
}
onMounted(() => document.addEventListener('pointerdown', pressedOutside, true))

/** Puts the focus back in the field, its text selected, as the shortcut does again while it shows. */
function focus(): void {
  field.value?.focus()
  field.value?.select()
}
defineExpose({ focus })

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', pressedOutside, true)
  clearTimeout(pause)
  search.stop()
})

/** The selected row: the selected file's, or the first. */
const current = computed(() => {
  const path = selectedPath.value
  return path === null ? 0 : Math.max(0, files.value.findIndex((file) => file.path === path))
})
function select(index: number): void {
  selectedPath.value = files.value[index]?.path ?? null
}

const scrollArea = ref<InstanceType<typeof ScrollArea> | null>(null)
const phone = usePhone()
/** A row's height: its name, and the line under it when its text was found. */
function rowPx(file: FoundFile): number {
  if (phone.value)
    return file.line ? 64 : 44
  return file.line ? 52 : 32
}
const virtualizer = useVirtualizer(computed(() => ({
  count: files.value.length,
  getScrollElement: () => scrollArea.value?.el ?? null,
  estimateSize: (index: number) => rowPx(files.value[index]!),
  getItemKey: (index: number) => files.value[index]?.path ?? index,
  overscan: 8,
})))
const drawn = computed(() => virtualizer.value.getVirtualItems().flatMap((item) => {
  const file = files.value[item.index]
  return file ? [{ file, index: item.index, start: item.start, size: item.size }] : []
}))

function move(step: 1 | -1): void {
  if (files.value.length === 0)
    return
  const next = stepHighlight(current.value, step, files.value.length)
  select(next)
  virtualizer.value.scrollToIndex(next, { align: 'auto' })
}

function openFile(file: FoundFile | undefined): void {
  if (file)
    emit('open', joinPath(props.root, file.path), file.line?.number ?? null)
}

// The field's keys; TextInput keeps an input method's keys to itself.
function keydown(event: KeyboardEvent): void {
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    move(event.key === 'ArrowDown' ? 1 : -1)
  } else if (event.key === 'Enter') {
    event.preventDefault()
    // The rows of another text, shown until this one's come, are not what Return opens.
    if (search.foundFor.value === text.value)
      openFile(files.value[current.value])
  } else if (event.key === 'Escape') {
    event.preventDefault()
    emit('close')
  }
}

function fileName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}

function folderOf(path: string): string {
  const slash = path.lastIndexOf('/')
  return slash < 0 ? '' : path.slice(0, slash)
}

function rangesOf(ranges: readonly FileSearchRange[]): number[][] {
  return ranges.map((range) => [range.start, range.end])
}

/** The characters a line shows before its first match, so the match stays in sight. */
const LEAD = 24

/** A line as its row shows it: from a little before its first match, cut with `…`, and its matches there. */
function shownLine(line: NonNullable<FoundFile['line']>): { text: string, ranges: number[][] } {
  const first = line.matches[0]?.start ?? 0
  let cut = first > LEAD + 1 ? first - LEAD : 0
  // Never between the two halves of a character outside the BMP, as an emoji.
  const unit = line.text.charCodeAt(cut)
  if (cut > 0 && unit >= 0xDC00 && unit <= 0xDFFF)
    cut -= 1
  const shown = cut === 0 ? line.text : `…${line.text.slice(cut)}`
  const shift = cut === 0 ? 0 : cut - 1
  return { text: shown, ranges: line.matches.map((match) => [match.start - shift, match.end - shift]) }
}

/** What the list says in place of rows, or none while it has rows to show. */
const status = computed(() => {
  const now = state.value
  if (now.kind === 'failed')
    return { status: 'failed' as const, label: t('search.failed'), detail: errorMessage(now.error) }
  if (files.value.length > 0)
    return null
  if (now.kind === 'searching')
    return { status: 'loading' as const, label: t('search.searching') }
  if (now.kind === 'done')
    return { status: 'note' as const, label: t('search.nothing') }
  return null
})
const listed = computed(() => text.value.trim() !== '' && (status.value !== null || files.value.length > 0))
</script>

<template>
  <!-- Over the whole view, letting presses through to it but on the field. -->
  <div class="pointer-events-none absolute inset-0 z-30 flex items-start justify-center">
    <div
      ref="card"
      role="dialog"
      :aria-label="t('search.title')"
      class="overlay-shell pointer-events-auto mt-2 flex max-h-[min(30rem,calc(100%-1rem))] w-[min(36rem,calc(100%-1rem))] min-h-0 flex-col overflow-hidden rounded-xl"
    >
      <div class="shrink-0 p-2">
        <TextInput
          v-model="text"
          search
          :placeholder="t('search.placeholder')"
          :aria-label="t('search.placeholder')"
          role="combobox"
          aria-autocomplete="list"
          :aria-controls="listId"
          :aria-expanded="files.length > 0"
          :aria-activedescendant="files.length ? `${listId}-${current}` : undefined"
          ref="field"
          data-file-search
          focused
          asked
          @keydown="keydown"
        >
          <template #prefix><Search :size="ICON_PX.in24" /></template>
        </TextInput>
      </div>
      <div v-if="listed" class="flex min-h-0 flex-col border-t border-line">
        <RegionStatus
          v-if="status"
          :status="status.status"
          :icon="status.status === 'note' ? SearchX : undefined"
          :label="status.label"
          :detail="status.status === 'failed' ? status.detail : null"
          :on-retry="status.status === 'failed' ? () => search.run(source, root, text) : undefined"
        />
        <ScrollArea v-else ref="scrollArea" class="min-h-0" viewport-class="p-1.5">
          <div
            :id="listId"
            role="listbox"
            :aria-label="t('search.results')"
            class="relative"
            :style="{ height: `${virtualizer.getTotalSize()}px` }"
          >
            <div
              v-for="{ file, index, start, size } in drawn"
              :id="`${listId}-${index}`"
              :key="file.path"
              role="option"
              :aria-selected="index === current"
              class="absolute inset-x-0 top-0 flex cursor-default select-none flex-col justify-center gap-0.5 rounded-md px-2"
              :class="index === current ? 'bg-active' : ''"
              :style="{ transform: `translateY(${start}px)`, height: `${size}px` }"
              @mousemove="select(index)"
              @click="openFile(file)"
            >
              <div class="flex min-w-0 items-center gap-1.5">
                <FileIcon class="shrink-0" :name="fileName(file.path)" :is-directory="false" />
                <span class="min-w-0 truncate text-chrome text-fg-emphasis"><HighlightText :text="fileName(file.path)" :ranges="rangesOf(file.name)" /></span>
                <span v-if="folderOf(file.path)" class="min-w-0 flex-1 truncate text-detail text-fg-subtle" :title="file.path">{{ folderOf(file.path) }}</span>
              </div>
              <p v-if="file.line" class="min-w-0 truncate whitespace-pre pl-[22px] text-explanation leading-5 text-fg-muted" :title="file.line.text">
                <HighlightText v-bind="shownLine(file.line)" />
              </p>
            </div>
          </div>
        </ScrollArea>
        <p v-if="state.kind === 'done' && state.limitReached" class="shrink-0 border-t border-line px-3.5 py-2 text-detail text-fg-subtle">
          {{ t('search.limited', { count: formatCount(state.files) }, state.files) }}
        </p>
      </div>
    </div>
  </div>
</template>

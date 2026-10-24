<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { CaseSensitive, Regex, WholeWord } from '@lucide/vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import Checkbox from '../ui/Checkbox.vue'
import FoldChevron from '../ui/FoldChevron.vue'
import HighlightText from '../ui/HighlightText.vue'
import IconButton from '../ui/IconButton.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import InlineError from '../ui/InlineError.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { usePhone } from '../ui/phone'
import { errorMessage } from '../infra/errors'
import { formatCount } from '../ui/numbers'
import FileIcon from './FileIcon.vue'
import { TextSearch, globsOf, type FoundFile } from './file-search'
import { joinPath } from './paths'
import { TREE_ROW_PHONE_PX, TREE_ROW_PX } from './tree'
import { t } from './locales/text'
import { FileBrowserError, type FileBrowserSource, type FileSearchMatch } from './types'

/**
 * Search in Files: the text of every file under `root` on the source's Host,
 * searched when Enter is pressed (`crates/runner-host/src/search.rs`).
 *
 * - The query's field holds three toggles: Match Case, Match Whole Word and
 *   Use Regular Expression. A row under it shows globs of the files to
 *   include and to exclude, separated by commas, and whether files
 *   `.gitignore` leaves out are searched too.
 * - Enter searches, and stops a search that runs. The files found show as the
 *   Host finds them, each in the tree's order with its count, and under it a
 *   row for each match, as VS Code lists them: its line's number and the part
 *   of the line around it, the match marked. A click on a file's row folds
 *   it; one on a match opens the file at its line.
 * - Down from the field moves into the results, where Up, Down, Home and End
 *   move, Left folds a file or goes to its row, Right unfolds it, and Enter
 *   acts as a click does.
 * - What a search found stays while the view switches to its tree and back,
 *   until another search, another root or another source, as when the
 *   conversation moves to another Host. Leaving the view stops the search.
 * - Only the rows in view are drawn: a search that reaches its limit lists
 *   ten thousand matches.
 */
const props = defineProps<{
  source: FileBrowserSource
  /** The directory searched, by absolute path. */
  root: string
}>()

const emit = defineEmits<{
  /** A match found: its file, by absolute path, and its line, 1-based. */
  open: [path: string, line: number]
}>()

const pattern = ref('')
const caseSensitive = ref(false)
const wholeWord = ref(false)
const regex = ref(false)
const filtersOpen = ref(false)
const include = ref('')
const exclude = ref('')
const includeIgnored = ref(false)

const search = new TextSearch()
const files = search.files
const state = search.state

/** The files folded, by path, until another search. */
const folded = shallowRef(new Set<string>())

watch([() => props.source, () => props.root], () => {
  search.clear()
  folded.value = new Set()
})

onBeforeUnmount(() => search.stop())

function run(): void {
  if (pattern.value === '')
    return
  folded.value = new Set()
  cursor.value = null
  void search.run(props.source, props.root, {
    pattern: pattern.value,
    caseSensitive: caseSensitive.value,
    wholeWord: wholeWord.value,
    regex: regex.value,
    include: globsOf(include.value),
    exclude: globsOf(exclude.value),
    includeIgnored: includeIgnored.value,
  })
}

const failure = computed(() => {
  const current = state.value
  if (current.kind !== 'failed')
    return null
  const error = current.error
  return {
    message: errorMessage(error),
    requestId: error instanceof FileBrowserError ? error.requestId : null,
    pattern: error instanceof FileBrowserError && error.kind === 'invalid-pattern',
  }
})

/** How many matches in how many files, as the summary says it. */
function found(matches: number, count: number): string {
  const results = t('search.resultCount', { count: formatCount(matches) }, matches)
  return t('search.inFiles', { results, count: formatCount(count) }, count)
}

/** What the line under the field says of the search. */
const summary = computed(() => {
  const current = state.value
  const shown = files.value
  const matches = shown.reduce((sum, file) => sum + file.matches.length, 0)
  switch (current.kind) {
    case 'idle':
      return shown.length === 0 ? '' : found(matches, shown.length)
    case 'searching':
      return shown.length === 0 ? t('search.searching') : found(matches, shown.length)
    case 'done':
      if (current.files === 0)
        return t('search.nothing')
      return current.limitReached
        ? t('search.limited', { found: found(current.matches, current.files) })
        : found(current.matches, current.files)
    case 'failed':
      return ''
  }
})

/** A row of the results: a file, or one of its matches. */
type ResultRow =
  | { kind: 'file', id: string, file: FoundFile }
  | { kind: 'match', id: string, file: FoundFile, match: FileSearchMatch }

const rows = computed<ResultRow[]>(() => {
  const shown: ResultRow[] = []
  for (const file of files.value) {
    shown.push({ kind: 'file', id: `f:${file.path}`, file })
    if (folded.value.has(file.path))
      continue
    file.matches.forEach((match, index) => {
      shown.push({ kind: 'match', id: `m:${index}:${file.path}`, file, match })
    })
  }
  return shown
})

/** The row the keyboard is on, by id. */
const cursor = ref<string | null>(null)
const list = ref<HTMLElement | null>(null)
const input = ref<InstanceType<typeof TextInput> | null>(null)
const scrollArea = ref<InstanceType<typeof ScrollArea> | null>(null)
const phone = usePhone()
const rowPx = computed(() => (phone.value ? TREE_ROW_PHONE_PX : TREE_ROW_PX))
const virtualizer = useVirtualizer(computed(() => ({
  count: rows.value.length,
  getScrollElement: () => scrollArea.value?.el ?? null,
  estimateSize: () => rowPx.value,
  getItemKey: (index: number) => rows.value[index]?.id ?? index,
  overscan: 10,
})))
/** The rows in view, each with its place in the list. */
const drawn = computed(() => virtualizer.value.getVirtualItems().flatMap((item) => {
  const row = rows.value[item.index]
  return row ? [{ row, start: item.start }] : []
}))

function fold(file: FoundFile, folds = !folded.value.has(file.path)): void {
  const next = new Set(folded.value)
  if (folds)
    next.add(file.path)
  else
    next.delete(file.path)
  folded.value = next
}

function activate(row: ResultRow): void {
  cursor.value = row.id
  if (row.kind === 'file')
    fold(row.file)
  else
    emit('open', joinPath(props.root, row.file.path), row.match.line)
}

function fileName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}

function folderOf(path: string): string {
  const slash = path.lastIndexOf('/')
  return slash < 0 ? '' : path.slice(0, slash)
}

/** The characters a row shows before its match, so the match stays in sight in a narrow tree. */
const LEAD = 12

/** A match as its row shows it: from a little before the match, cut with `…`, and the match there. */
function shownMatch(match: FileSearchMatch): { text: string, ranges: number[][] } {
  let cut = match.start > LEAD + 1 ? match.start - LEAD : 0
  // Never between the two halves of a character outside the BMP, as an emoji.
  const unit = match.text.charCodeAt(cut)
  if (cut > 0 && unit >= 0xDC00 && unit <= 0xDFFF)
    cut -= 1
  const text = cut === 0 ? match.text : `…${match.text.slice(cut)}`
  const shift = cut === 0 ? 0 : cut - 1
  return { text, ranges: [[match.start - shift, match.end - shift]] }
}

function moveTo(index: number): void {
  const at = Math.max(0, Math.min(index, rows.value.length - 1))
  const row = rows.value[at]
  if (!row)
    return
  cursor.value = row.id
  virtualizer.value.scrollToIndex(at, { align: 'auto' })
}

function onListKey(event: KeyboardEvent): void {
  const index = rows.value.findIndex((row) => row.id === cursor.value)
  const row = rows.value[index]
  switch (event.key) {
    case 'ArrowDown':
      moveTo(index + 1)
      break
    case 'ArrowUp':
      if (index <= 0)
        input.value?.focus()
      else
        moveTo(index - 1)
      break
    case 'Home':
      moveTo(0)
      break
    case 'End':
      moveTo(rows.value.length - 1)
      break
    case 'ArrowLeft':
      if (row?.kind === 'file')
        fold(row.file, true)
      else if (row)
        moveTo(rows.value.findIndex((other) => other.id === `f:${row.file.path}`))
      break
    case 'ArrowRight':
      if (row?.kind === 'file')
        fold(row.file, false)
      break
    case 'Enter':
      if (row)
        activate(row)
      break
    default:
      return
  }
  event.preventDefault()
}

function intoResults(): void {
  if (rows.value.length === 0)
    return
  list.value?.focus()
  moveTo(Math.max(0, rows.value.findIndex((row) => row.id === cursor.value)))
}

defineExpose({
  /** Focuses the query with its text selected, as Search in Files does. */
  focus(): void {
    input.value?.focus()
    input.value?.select()
  },
})
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <div class="flex shrink-0 flex-col gap-1.5 px-2 pb-2 pt-1">
      <TextInput
        ref="input"
        v-model="pattern"
        size="sm"
        search
        literal
        data-file-search
        :placeholder="t('search.placeholder')"
        :invalid="failure?.pattern ?? false"
        @keydown.enter.prevent="run"
        @keydown.down.prevent="intoResults"
      >
        <template #suffix>
          <span class="-mr-1 flex shrink-0 items-center">
            <Tooltip :content="t('search.matchCase')">
              <IconButton :icon="CaseSensitive" size="xs" variant="ghost" :pressed="caseSensitive" :aria-label="t('search.matchCase')" @click="caseSensitive = !caseSensitive" />
            </Tooltip>
            <Tooltip :content="t('search.wholeWord')">
              <IconButton :icon="WholeWord" size="xs" variant="ghost" :pressed="wholeWord" :aria-label="t('search.wholeWord')" @click="wholeWord = !wholeWord" />
            </Tooltip>
            <Tooltip :content="t('search.regex')">
              <IconButton :icon="Regex" size="xs" variant="ghost" :pressed="regex" :aria-label="t('search.regex')" @click="regex = !regex" />
            </Tooltip>
          </span>
        </template>
      </TextInput>
      <button
        type="button"
        class="flex h-5 w-fit select-none items-center gap-0.5 rounded-sm pr-1 text-[12px] text-fg-subtle hover:text-fg-body"
        :aria-expanded="filtersOpen"
        @click="filtersOpen = !filtersOpen"
      >
        <FoldChevron :open="filtersOpen" :size="ICON_PX.in20" />
        {{ t('search.filters') }}
      </button>
      <template v-if="filtersOpen">
        <TextInput v-model="include" size="sm" literal :placeholder="t('search.include')" @keydown.enter.prevent="run" />
        <TextInput v-model="exclude" size="sm" literal :placeholder="t('search.exclude')" @keydown.enter.prevent="run" />
        <Checkbox v-model="includeIgnored" :label="t('search.includeIgnored')" />
      </template>
      <InlineError v-if="failure" :message="failure.message" :request-id="failure.requestId" />
      <div v-else-if="summary || state.kind === 'searching'" class="flex min-h-5 items-center gap-1.5 text-[12px] text-fg-subtle" aria-live="polite">
        <IndeterminateSpinner v-if="state.kind === 'searching'" :size="12" :stroke-width="1.5" />
        <span>{{ summary }}</span>
      </div>
    </div>
    <ScrollArea ref="scrollArea" class="min-h-0 flex-1" viewport-class="px-1 pb-2">
      <div
        ref="list"
        role="tree"
        :aria-label="t('search.results')"
        :aria-activedescendant="cursor ? `file-search-${cursor}` : undefined"
        tabindex="-1"
        class="relative outline-none"
        :style="{ height: `${virtualizer.getTotalSize()}px` }"
        @keydown="onListKey"
      >
        <div
          v-for="{ row, start } in drawn"
          :id="`file-search-${row.id}`"
          :key="row.id"
          role="treeitem"
          :aria-level="row.kind === 'file' ? 1 : 2"
          :aria-expanded="row.kind === 'file' ? !folded.has(row.file.path) : undefined"
          class="absolute inset-x-0 top-0 flex h-7 cursor-default select-none items-center gap-1 rounded-md pr-2 text-chrome text-fg-body hover:bg-hover phone:h-11 phone:text-[16px]"
          :class="row.id === cursor ? 'bg-active text-fg-emphasis' : ''"
          :style="{ transform: `translateY(${start}px)`, paddingLeft: row.kind === 'file' ? '4px' : '24px' }"
          @click="activate(row)"
        >
          <template v-if="row.kind === 'file'">
            <span class="flex size-4 shrink-0 items-center justify-center text-fg-faint">
              <FoldChevron :open="!folded.has(row.file.path)" :size="ICON_PX.in20" />
            </span>
            <FileIcon class="shrink-0" :name="fileName(row.file.path)" :is-directory="false" />
            <span class="min-w-0 truncate" :title="row.file.path">
              {{ fileName(row.file.path) }}
              <span v-if="folderOf(row.file.path)" class="ml-1 text-fg-faint">{{ folderOf(row.file.path) }}</span>
            </span>
            <span class="ml-auto shrink-0 pl-2 text-[11px] tabular-nums text-fg-faint">{{ formatCount(row.file.matches.length) }}</span>
          </template>
          <template v-else>
            <span class="w-8 shrink-0 pr-1.5 text-right text-[11px] tabular-nums text-fg-faint">{{ row.match.line }}</span>
            <span class="min-w-0 truncate whitespace-pre" :title="row.match.text">
              <HighlightText v-bind="shownMatch(row.match)" />
            </span>
          </template>
        </div>
      </div>
    </ScrollArea>
  </div>
</template>

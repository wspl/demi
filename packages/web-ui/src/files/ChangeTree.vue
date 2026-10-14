<script setup lang="ts">
import LineCounts from './LineCounts.vue'
import { computed, ref } from 'vue'
import { RefreshCw } from '@lucide/vue'
import HighlightText from '../ui/HighlightText.vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import GitStatusLetter from './GitStatusLetter.vue'
import Tree from './Tree.vue'
import { changeTreeRows, type ChangeTreeFile, type ChangeTreeRow } from './changes'
import { gitMark } from './git-status'
import { baseName } from '@demicodes/utils'
import type { HeadlineText } from '../ui/ui-text'

/**
 * The changed files beside the diff as a tree, in both of the Change view's
 * modes, as VS Code's Source Control shows changes as a tree: the folders
 * that hold a changed file, folders first and names in order, a folder
 * whose only content is one folder joined with it into one row
 * (`src/web/app`), every folder open until it is folded (`changeTreeRows`).
 * A file ends its row with its line counts when they are known and, in
 * Uncommitted mode, the letter VS Code's Git marks it with, from git's
 * status of it (`GitStatusLetter`): U untracked, A added, M modified, D
 * deleted, R renamed, C copied, T type changed, ! in conflict. A deleted
 * file's name is struck through, and a renamed one's tooltip names where it
 * came from. The caption row names the workspace and, at its end, holds the
 * control that lists the changes again when the host offers one; it turns
 * while a list is on its way. A list cut short says so under its last row.
 * A click on a file selects it, as Enter does on the row the keyboard is
 * on; typing a name moves the keyboard to it (`Tree`).
 */
const props = defineProps<{
  files: readonly ChangeTreeFile[]
  /** The workspace, named at the top. */
  root: string
  /** What heads the tree in place of the root directory's name. */
  rootName?: string
  /** The selected file, by its key. */
  selected: string | null
  /** What the tree says when there are no files. */
  emptyText: HeadlineText
  /** Lists the changes again; no control without it. */
  refresh?: () => void
  /** A new list is on its way. */
  refreshing?: boolean
  /** The list stopped at its limit. */
  truncated?: boolean
}>()

const emit = defineEmits<{
  select: [key: string]
}>()

const folded = ref(new Set<string>())
const rows = computed(() => changeTreeRows(props.files, folded.value))
const rootName = computed(() => props.rootName ?? (baseName(props.root) || '/'))

function toggle(path: string): void {
  const next = new Set(folded.value)
  if (next.has(path)) {
    next.delete(path)
  } else {
    next.add(path)
  }
  folded.value = next
}

function activate(row: ChangeTreeRow): void {
  if (row.isDirectory) {
    toggle(row.path)
  } else {
    emit('select', row.path)
  }
}

function struck(file: ChangeTreeFile | null): boolean {
  if (!file)
    return false
  return file.status === undefined ? file.kind === 'deleted' : gitMark(file.status)?.strike === true
}

function tooltip(row: ChangeTreeRow): string {
  const file = row.file
  if (!file)
    return row.path
  return file.from ? `${file.from} → ${file.path}` : file.path
}
</script>

<template>
  <Tree
    :rows="rows"
    :caption="rootName"
    :caption-title="root"
    :selected="selected"
    :tooltip="tooltip"
    @activate="activate"
  >
    <template #captionTrailing>
      <Tooltip v-if="refresh" content="Refresh" class="ml-2 shrink-0">
        <IconButton
          :icon="RefreshCw"
          size="xs"
          variant="ghost"
          aria-label="Refresh"
          spin-on-click
          :spinning="refreshing"
          @click="refresh()"
        />
      </Tooltip>
    </template>
    <template #name="{ row, highlight }">
      <TruncatedText :class="struck(row.file) ? 'deleted-name' : ''" :text="row.name">
        <HighlightText :text="row.name" :indexes="highlight" />
      </TruncatedText>
    </template>
    <template #trailing="{ row }">
      <span v-if="row.file && (row.file.lines || row.file.status !== undefined)" class="ml-auto flex shrink-0 items-center gap-2 pl-2">
        <LineCounts v-if="row.file.lines" :added="row.file.lines.added" :removed="row.file.lines.removed" />
        <GitStatusLetter v-if="row.file.status !== undefined" :status="row.file.status" />
      </span>
    </template>
    <template #empty>
      <div class="flex flex-1 select-none items-center justify-center px-4 py-10 text-center text-[13px] text-fg-subtle">
        {{ emptyText }}
      </div>
    </template>
    <template #after>
      <div v-if="truncated" class="select-none px-2 py-2 text-[11px] text-fg-faint">
        More files changed than the list holds.
      </div>
    </template>
  </Tree>
</template>

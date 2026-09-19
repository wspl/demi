<script setup lang="ts">
import { computed, ref } from 'vue'
import { RefreshCw } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import Tree from './Tree.vue'
import { changeTreeRows, type ChangeFile, type ChangeSetSource, type ChangeTreeRow } from './changes'
import { baseName } from './paths'

/**
 * The changed files as a tree beside the diff, on a `Tree`: directories on
 * the way to them fold and unfold. A file ends its row with its line counts
 * and then the letter VS Code's source control gives its change, in that
 * change's color: A added, M modified, D deleted, R renamed, whose tooltip
 * names the change (and a rename's old path); a deleted file's name is
 * struck through. The caption row names the workspace and, at its end,
 * holds the control that lists the changes again when the source can; it
 * turns while a list is on its way. A list cut short says so under its last
 * row. A click on a file selects it.
 */
const props = defineProps<{
  source: ChangeSetSource
  /** The workspace, named at the top. */
  root: string
  /** What heads the tree in place of the root directory's name. */
  rootName?: string
  /** The selected file, by path relative to the workspace. */
  selected: string | null
  /** What the tree says when there are no files. */
  emptyText: string
}>()

const emit = defineEmits<{
  select: [path: string]
}>()

/** Each kind of change as its row ends: VS Code's letter, the word for it, and its color. */
const CHANGE_MARKS: Record<ChangeFile['kind'], { letter: string; word: string; tone: string }> = {
  added: { letter: 'A', word: 'Added', tone: 'text-on-success' },
  modified: { letter: 'M', word: 'Modified', tone: 'text-on-warning' },
  deleted: { letter: 'D', word: 'Deleted', tone: 'text-on-danger' },
  renamed: { letter: 'R', word: 'Renamed', tone: 'text-on-success' },
}

/** What the letter's tooltip says: the change, and where a renamed file came from. */
function changeHint(change: ChangeFile): string {
  return change.kind === 'renamed' && change.from ? `Renamed from ${change.from}` : CHANGE_MARKS[change.kind].word
}

const folded = ref(new Set<string>())
const files = computed(() => props.source.files)
const rows = computed(() => changeTreeRows(files.value, folded.value))
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
</script>

<template>
  <Tree
    :rows="rows"
    :caption="rootName"
    :caption-title="root"
    :selected="selected"
    @activate="activate"
  >
    <template #captionTrailing>
      <Tooltip v-if="source.refresh" content="Refresh" class="ml-2 shrink-0">
        <IconButton
          :icon="RefreshCw"
          size="xs"
          variant="ghost"
          aria-label="Refresh"
          spin-on-click
          :spinning="source.refreshing"
          @click="source.refresh?.()"
        />
      </Tooltip>
    </template>
    <template #name="{ row }">
      <span class="truncate" :class="row.change?.kind === 'deleted' ? 'line-through text-fg-muted' : ''">{{ row.name }}</span>
    </template>
    <template #trailing="{ row }">
      <span v-if="row.change" class="ml-auto flex shrink-0 items-center gap-2 pl-2">
        <!-- Counts as on the changed-file pills: added in green, removed in red, zero omitted. -->
        <span class="inline-flex gap-1 font-mono text-[11px] tabular-nums">
          <span v-if="row.change.added > 0" class="text-on-success">+{{ row.change.added }}</span>
          <span v-if="row.change.removed > 0" class="text-on-danger">−{{ row.change.removed }}</span>
        </span>
        <!-- One letter wide whatever the letter, so the letters line up down the tree. -->
        <Tooltip :content="changeHint(row.change)" class="flex w-3 justify-center">
          <span class="text-[12px] font-medium" :class="CHANGE_MARKS[row.change.kind].tone" :aria-label="changeHint(row.change)">{{
            CHANGE_MARKS[row.change.kind].letter
          }}</span>
        </Tooltip>
      </span>
    </template>
    <template #empty>
      <div class="flex flex-1 select-none items-center justify-center px-4 py-10 text-center text-[13px] text-fg-subtle">
        {{ emptyText }}
      </div>
    </template>
    <template #after>
      <div v-if="source.truncated" class="select-none px-2 py-2 text-[11px] text-fg-faint">
        More files changed than the list holds.
      </div>
    </template>
  </Tree>
</template>

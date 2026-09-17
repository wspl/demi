<script setup lang="ts">
import { computed, ref } from 'vue'
import { FileDiff, Globe, PanelRightClose } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from '../files/FileIcon.vue'
import ChangeView from '../files/ChangeView.vue'
import FileView from '../files/FileView.vue'
import { callChangeSource, emptyChangeSet, type ReadCallChange, type ChangeMode, type ChangeSetSource, type ChangeSources } from '../files/changes'
import { joinPath, normalizePath } from '../files/paths'
import type { FileBrowserSource } from '../files/types'
import BrowserPanel from './BrowserPanel.vue'
import { changeTabPath, workTabTitle, type BrowserWorkTab, type ChangeWorkTab, type WorkTab } from './work-panel'

/** Fixed work-panel sections; the host retains each conversation's selections. */
const props = defineProps<{
  tabs: readonly WorkTab[]
  activeId: string | null
  readCallChange?: ReadCallChange
  historyRoot?: string
  workspace?: { source: FileBrowserSource; root: string; name?: string; changes?: ChangeSetSource }
}>()
const emit = defineEmits<{
  select: [id: string]
  showChange: [id: string, mode: ChangeMode, path: string | null, selection?: { call: ChangeWorkTab['call']; edit: number }]
  updateBrowser: [tab: BrowserWorkTab]
  open: [path: string]
  back: [id: string]
  forward: [id: string]
  close: []
}>()

const treeOpen = ref(true)
const active = computed(() => props.tabs.find((tab) => tab.id === props.activeId) ?? null)
const changes = computed<ChangeSources>(() => {
  const call = active.value?.kind === 'change' ? active.value.call : null
  return {
    uncommitted: props.workspace?.changes ?? emptyChangeSet,
    conversation: call && props.readCallChange ? callChangeSource(call, props.readCallChange) : null,
  }
})
const totals = computed(() => changes.value.uncommitted.files.reduce(
  (sum, file) => ({ added: sum.added + file.added, removed: sum.removed + file.removed }),
  { added: 0, removed: 0 },
))

function select(tab: WorkTab): void {
  if (tab.kind === 'change') {
    emit('showChange', tab.id, 'uncommitted', tab.uncommitted)
  }
  emit('select', tab.id)
}

/** Resolve the changed file within the selected change mode. */
function changeSelection(tab: ChangeWorkTab): string | null {
  return changeTabPath(tab, tab.mode, changes.value.uncommitted.files)
}

function absolutePath(path: string): string {
  return path.startsWith('/') ? path : joinPath(props.workspace?.root ?? '/', path)
}

function openFromTree(path: string): void {
  const root = normalizePath(props.workspace?.root ?? '/')
  const relative = path.startsWith(`${root}/`) ? path.slice(root.length + 1) : path
  emit('open', relative)
}
</script>

<template>
  <aside class="flex h-full min-w-0 flex-col overflow-hidden border-l border-line bg-surface text-fg">
    <div class="flex h-11 shrink-0 items-center gap-1 pl-2 pr-3">
      <div role="tablist" aria-label="Work panel" class="flex min-w-0 flex-1 items-center gap-1">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          type="button"
          role="tab"
          :aria-selected="tab.id === activeId"
          :title="tab.kind === 'file' ? tab.path || 'File' : workTabTitle(tab)"
          class="flex h-7 min-w-0 items-center gap-1.5 rounded-md px-2 text-chrome hover:bg-surface-base hover:text-fg"
          :class="[tab.kind === 'file' ? 'shrink' : 'shrink-0', tab.id === activeId ? 'bg-surface-base text-fg-emphasis' : 'text-fg-subtle']"
          @click="select(tab)"
        >
          <FileIcon v-if="tab.kind === 'file'" :name="tab.path" :is-directory="false" :size="ICON_PX.markIn28" class="shrink-0" />
          <FileDiff v-else-if="tab.kind === 'change'" :size="ICON_PX.markIn28" class="shrink-0" />
          <Globe v-else :size="ICON_PX.markIn28" class="shrink-0" />
          <span class="truncate whitespace-nowrap">{{ tab.kind === 'file' ? tab.path ? `File: ${workTabTitle(tab)}` : 'File' : workTabTitle(tab) }}</span>
          <span v-if="tab.kind === 'change'" class="flex shrink-0 gap-1 text-[11px] tabular-nums">
            <span class="text-on-success">+{{ totals.added }}</span>
            <span class="text-on-danger">−{{ totals.removed }}</span>
          </span>
        </button>
      </div>
      <Tooltip content="Close panel" class="shrink-0">
        <IconButton :icon="PanelRightClose" variant="ghost" aria-label="Close panel" @click="emit('close')" />
      </Tooltip>
    </div>
    <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <slot :tab="active">
        <BrowserPanel v-if="active?.kind === 'browser'" :tab="active" @update="emit('updateBrowser', $event)" />
        <!-- File navigation reuses the view and its unfolded tree. -->
        <FileView
          v-else-if="active?.kind === 'file' && workspace"
          v-model:tree="treeOpen"
          :source="workspace.source"
          :root="workspace.root"
          :root-name="workspace.name"
          :path="active.path ? absolutePath(active.path) : null"
          :can-back="active.back.length > 0"
          :can-forward="active.forward.length > 0"
          @open="openFromTree"
          @back="emit('back', active.id)"
          @forward="emit('forward', active.id)"
        />
        <ChangeView
          v-else-if="active?.kind === 'change'"
          v-model:tree="treeOpen"
          :mode="active.mode"
          :selected="changeSelection(active)"
          :changes="changes"
          :edit="active.edit"
          :root="workspace?.root ?? historyRoot ?? '/'"
          :root-name="workspace?.name"
          :can-back="active.back.length > 0"
          :can-forward="active.forward.length > 0"
          @update:mode="emit('showChange', active.id, $event, changeTabPath(active, $event))"
          @update:edit="emit('showChange', active.id, active.mode, changeSelection(active), { call: active.call, edit: $event })"
          @update:selected="emit('showChange', active.id, active.mode, $event)"
          @back="emit('back', active.id)"
          @forward="emit('forward', active.id)"
          @open="emit('open', $event)"
        />
        <div
          v-else
          class="flex flex-1 select-none flex-col items-center justify-center gap-1 text-[13px] text-fg-faint"
        >
          <template v-if="active?.kind === 'file'">
            <span>File</span>
            <span class="max-w-full truncate px-4 font-mono text-[11px]">{{ active.path }}</span>
          </template>
          <span v-else-if="active">Change</span>
          <span v-else>No files open</span>
        </div>
      </slot>
    </div>
  </aside>
</template>

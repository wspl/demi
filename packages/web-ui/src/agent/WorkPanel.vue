<script lang="ts">
import { ref } from 'vue'

// Markdown and SVG: what each view shows them as holds across files,
// conversations and panels for the page's lifetime (`file-previews.md`).
const fileMode = ref<'preview' | 'source'>('preview')
const changePresentation = ref<'diff' | 'preview'>('diff')
</script>

<script setup lang="ts">
import { computed } from 'vue'
import { File, FileDiff, PanelRightClose, Plus, X } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from '../files/FileIcon.vue'
import ChangeView from '../files/ChangeView.vue'
import FileView from '../files/FileView.vue'
import { TREE_WIDTH } from '../files/file-view'
import { callChangeSource, emptyChangeSet, type ReadCallChange, type ChangeMode, type ChangeSetSource, type ChangeSources } from '../files/changes'
import { joinPath, relativePath } from '../files/paths'
import type { FileBrowserSource } from '../files/types'
import TabItem from './TabItem.vue'
import TabStrip from './TabStrip.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import { appOverlayStore } from '../overlay/appOverlay'
import { tabsToClose, type TabCloseScope } from './tab-close'
import { changeTabPath, workTabTitle, type ChangeWorkTab, type WorkTab } from './work-panel'
import type { PanelState } from './panel-tabs'
import { resolvePanelTab, type PanelTabKind } from './panel-kinds/kind'

/**
 * The fixed Change and File views beside a strip of the user's tabs
 * (`web-application.md` § Work panel). The panel shows a tab through its
 * kind's registration and knows nothing else about it.
 */
const props = defineProps<{
  /** The fixed views' own state. They are not tabs; they only compete for the selection. */
  views: readonly WorkTab[]
  /** The selection and the user's tabs. */
  panel: PanelState
  kinds: readonly PanelTabKind[]
  readCallChange?: ReadCallChange
  historyRoot?: string
  workspace?: { source: FileBrowserSource; root: string; name?: string; changes?: ChangeSetSource }
}>()
const emit = defineEmits<{
  select: [selection: string]
  addTab: [kind: string, data: unknown]
  updateTab: [id: string, data: unknown]
  closeTabs: [ids: string[]]
  showChange: [id: string, mode: ChangeMode, path: string | null, selection?: { call: ChangeWorkTab['call']; edit: number }]
  open: [path: string]
  back: [id: string]
  forward: [id: string]
  close: []
}>()

// File and Change share their tree's visibility and width, so both hold across files and views.
const treeOpen = ref(true)
const treeWidth = ref<number>(TREE_WIDTH.default)
const active = computed(() => props.views.find((view) => view.id === props.panel.selection) ?? null)
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

/** Each tab with its kind and checked data, in the user's order. */
const tabs = computed(() => props.panel.tabs.map((tab) => resolvePanelTab(tab, props.kinds)))
const shownTab = computed(() => tabs.value.find((item) => item.tab.id === props.panel.selection) ?? null)
/** The kinds the strip's new-tab control offers. */
const creatable = computed(() => props.kinds.filter((kind) => kind.create))
const menuId = ref<string | null>(null)
const menu = useContextMenuOwner(() => {
  menuId.value = null
})

function openMenu(event: MouseEvent, id: string): void {
  menuId.value = id
  menu.open(event)
}

function closeScope(scope: TabCloseScope): void {
  if (menuId.value) {
    emit('closeTabs', tabsToClose(props.panel.tabs, menuId.value, scope))
  }
  menu.close()
}

/** Resolve the changed file within the selected change mode. */
function changeSelection(tab: ChangeWorkTab): string | null {
  return changeTabPath(tab, tab.mode, changes.value.uncommitted.files)
}

function absolutePath(path: string): string {
  return path.startsWith('/') ? path : joinPath(props.workspace?.root ?? '/', path)
}

function openFromTree(path: string): void {
  emit('open', relativePath(props.workspace?.root ?? '/', path))
}
</script>

<template>
  <aside class="flex h-full min-w-0 flex-col overflow-hidden border-l border-line bg-surface text-fg">
    <div class="flex h-11 shrink-0 items-center gap-1 pl-2 pr-3">
      <div class="flex shrink-0 items-center gap-1" role="group" aria-label="Work panel views">
        <button
          v-for="tab in views"
          :key="tab.id"
          type="button"
          :aria-pressed="tab.id === panel.selection"
          :title="tab.kind === 'file' ? tab.path || 'File' : 'Change'"
          class="flex h-7 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md px-1.5 text-chrome hover:bg-surface-base hover:text-fg"
          :class="tab.id === panel.selection ? 'bg-surface-base text-fg-emphasis' : 'text-fg-subtle'"
          @click="select(tab)"
        >
          <FileIcon v-if="tab.kind === 'file' && tab.path" :name="tab.path" :is-directory="false" :size="ICON_PX.markIn28" />
          <File v-else-if="tab.kind === 'file'" :size="ICON_PX.markIn28" />
          <FileDiff v-else :size="ICON_PX.markIn28" />
          <span>{{ tab.kind === 'file' ? tab.path ? `File: ${workTabTitle(tab)}` : 'File' : 'Change' }}</span>
          <span v-if="tab.kind === 'change'" class="flex items-center gap-0.5 text-[11px] tabular-nums">
            <span class="text-on-success">+{{ totals.added }}</span>
            <span class="text-on-danger">−{{ totals.removed }}</span>
          </span>
        </button>
      </div>
      <TabStrip class="min-w-0 flex-1" surface="raised">
        <TabItem
          v-for="item in tabs"
          :key="item.tab.id"
          :title="item.title"
          :is-active="item.tab.id === panel.selection"
          tabindex="0"
          @pointerdown="emit('select', item.tab.id)"
          @keydown.enter="emit('select', item.tab.id)"
          @keydown.space.prevent="emit('select', item.tab.id)"
          @contextmenu="openMenu($event, item.tab.id)"
          @close="emit('closeTabs', [item.tab.id])"
        >
          <template v-if="item.kind" #mark><component :is="item.kind.mark" :data="item.data" /></template>
        </TabItem>
        <!-- A plain plus says enough beside tabs of the one kind it makes; otherwise each kind shows its own icon. -->
        <template v-if="creatable.length > 0" #trailing>
          <Tooltip
            v-for="kind in creatable"
            :key="kind.kind"
            :content="kind.create!.label"
            class="ml-1 shrink-0"
          >
            <IconButton
              :icon="tabs.length > 0 && creatable.length === 1 ? Plus : kind.create!.icon"
              size="sm"
              variant="ghost"
              :aria-label="kind.create!.label"
              @click="emit('addTab', kind.kind, kind.create!.data())"
            />
          </Tooltip>
        </template>
      </TabStrip>
      <Tooltip content="Close panel" class="shrink-0">
        <IconButton :icon="PanelRightClose" variant="ghost" aria-label="Close panel" @click="emit('close')" />
      </Tooltip>
    </div>
    <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <slot :tab="active">
        <component
          :is="shownTab.kind.content"
          v-if="shownTab?.kind"
          :key="shownTab.tab.id"
          :tab-id="shownTab.tab.id"
          :data="shownTab.data"
          shown
          @update="emit('updateTab', shownTab.tab.id, $event)"
          @close="emit('closeTabs', [shownTab.tab.id])"
        />
        <div
          v-else-if="shownTab"
          class="flex flex-1 select-none flex-col items-center justify-center gap-1 px-6 text-center text-[13px] text-fg-faint"
        >
          <span>This tab cannot be shown.</span>
          <span class="font-mono text-[11px]">{{ shownTab.tab.kind }}</span>
        </div>
        <!-- File navigation reuses the view and its unfolded tree. -->
        <FileView
          v-else-if="active?.kind === 'file' && workspace"
          v-model:tree="treeOpen"
          v-model:tree-width="treeWidth"
          v-model:mode="fileMode"
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
          v-model:tree-width="treeWidth"
          v-model:presentation="changePresentation"
          :mode="active.mode"
          :selected="changeSelection(active)"
          :changes="changes"
          :edit="active.edit"
          :root="workspace?.root ?? historyRoot ?? '/'"
          :root-name="workspace?.name"
          :contents="workspace?.source.contents"
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
    <Popover
      :overlay-store="appOverlayStore"
      :is-open="menu.isOpen.value"
      :anchor-x="menu.anchorX.value"
      :anchor-y="menu.anchorY.value"
      :anchor-context-el="menu.anchorContextEl.value"
      :offset="0"
      @close="menu.close"
    >
      <Menu>
        <MenuItem :icon="X" label="Close" @select="closeScope('self')" />
        <MenuItem
          v-for="item in [{ scope: 'others', label: 'Close others' }, { scope: 'right', label: 'Close to the right' }, { scope: 'left', label: 'Close to the left' }] as const"
          :key="item.scope"
          :label="item.label"
          :disabled="!menuId || tabsToClose(panel.tabs, menuId, item.scope).length === 0"
          @select="closeScope(item.scope)"
        />
      </Menu>
    </Popover>
  </aside>
</template>

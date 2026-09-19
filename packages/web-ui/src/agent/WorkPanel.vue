<script lang="ts">
import { ref } from 'vue'

// Markdown and SVG: what each view shows them as holds across files,
// conversations and panels for the page's lifetime (`file-previews.md`).
const fileMode = ref<'preview' | 'source'>('preview')
const changePresentation = ref<'diff' | 'preview'>('diff')
</script>

<script setup lang="ts">
import { computed } from 'vue'
import { File, FileDiff, Globe, MonitorDot, PanelRightClose, Plus, X } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import { GlobePlus } from '../ui/GlobePlus'
import { EXPOSE_ICON } from '../hosts/icons'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from '../files/FileIcon.vue'
import ChangeView from '../files/ChangeView.vue'
import FileView from '../files/FileView.vue'
import { TREE_WIDTH } from '../files/file-view'
import { callChangeSource, emptyChangeSet, type ReadCallChange, type ChangeMode, type ChangeSetSource, type ChangeSources } from '../files/changes'
import { joinPath, relativePath } from '../files/paths'
import type { FileBrowserSource } from '../files/types'
import BrowserPanel from './BrowserPanel.vue'
import HostBrowserPanel from '../browser/HostBrowserPanel.vue'
import type { LiveSession } from '../browser/session'
import TabItem from './TabItem.vue'
import TabStrip from './TabStrip.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import { appOverlayStore } from '../overlay/appOverlay'
import { tabsToClose, type TabCloseScope } from './tab-close'
import { changeTabPath, workTabTitle, type BrowserWorkTab, type ChangeWorkTab, type WorkTab } from './work-panel'

/** Fixed view buttons alongside a strip of removable browser tabs. */
const props = defineProps<{
  tabs: readonly WorkTab[]
  activeId: string | null
  readCallChange?: ReadCallChange
  historyRoot?: string
  /** The conversation browser's live view, when the page has one. */
  live?: LiveSession
  workspace?: { source: FileBrowserSource; root: string; name?: string; changes?: ChangeSetSource }
}>()
const emit = defineEmits<{
  select: [id: string]
  addBrowser: []
  closeTabs: [ids: string[]]
  showChange: [id: string, mode: ChangeMode, path: string | null, selection?: { call: ChangeWorkTab['call']; edit: number }]
  updateBrowser: [tab: BrowserWorkTab]
  open: [path: string]
  back: [id: string]
  forward: [id: string]
  close: []
}>()

// File and Change share their tree's visibility and width, so both hold across files and views.
const treeOpen = ref(true)
const treeWidth = ref<number>(TREE_WIDTH.default)
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

const fixedViews = computed(() => props.tabs.filter((tab) => tab.kind === 'file' || tab.kind === 'change'))
const hostTabs = computed(() => props.tabs.filter((tab) => tab.kind === 'host'))
/** The live tab the active Host tab shows, as the view reports it now. */
const hostTab = computed(() => {
  const tab = active.value
  return tab?.kind === 'host'
    ? props.live?.state.tabs.find((live) => live.id === tab.tab.id) ?? null
    : null
})
const browserTabs = computed(() => props.tabs.filter((tab) => tab.kind === 'browser'))
const menuId = ref<string | null>(null)
const menu = useContextMenuOwner(() => {
  menuId.value = null
})

function openMenu(event: MouseEvent, tab: WorkTab): void {
  if (tab.kind !== 'browser') {
    return
  }
  menuId.value = tab.id
  menu.open(event)
}

function closeScope(scope: TabCloseScope): void {
  if (menuId.value) {
    emit('closeTabs', tabsToClose(browserTabs.value, menuId.value, scope))
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
          v-for="tab in fixedViews"
          :key="tab.id"
          type="button"
          :aria-pressed="tab.id === activeId"
          :title="tab.kind === 'file' ? tab.path || 'File' : 'Change'"
          class="flex h-7 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md px-1.5 text-chrome hover:bg-surface-base hover:text-fg"
          :class="tab.id === activeId ? 'bg-surface-base text-fg-emphasis' : 'text-fg-subtle'"
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
        <!-- The conversation browser's own tabs, shown live. -->
        <TabItem
          v-for="tab in hostTabs"
          :key="tab.id"
          :title="workTabTitle(tab)"
          :is-active="tab.id === activeId"
          tabindex="0"
          @pointerdown="emit('select', tab.id)"
          @keydown.enter="emit('select', tab.id)"
          @keydown.space.prevent="emit('select', tab.id)"
          @close="tab.kind === 'host' && live?.closeTab(tab.tab.id)"
        >
          <template #mark><MonitorDot :size="ICON_PX.markIn28" /></template>
        </TabItem>
        <span v-if="live" class="mr-1 flex shrink-0">
          <Tooltip content="New tab in the conversation's browser">
            <IconButton :icon="GlobePlus" size="sm" variant="ghost" aria-label="New Host browser tab" @click="live.openTab()" />
          </Tooltip>
        </span>
        <TabItem
          v-for="tab in browserTabs"
          :key="tab.id"
          :title="tab.title"
          :is-active="tab.id === activeId"
          tabindex="0"
          @pointerdown="select(tab)"
          @keydown.enter="select(tab)"
          @keydown.space.prevent="select(tab)"
          @contextmenu="openMenu($event, tab)"
          @close="emit('closeTabs', [tab.id])"
        >
          <template #mark><component :is="tab.expose ? EXPOSE_ICON : Globe" :size="ICON_PX.markIn28" /></template>
        </TabItem>
        <template #trailing>
          <Tooltip content="New browser tab" class="ml-1 shrink-0">
            <IconButton :icon="browserTabs.length > 0 ? Plus : GlobePlus" size="sm" variant="ghost" aria-label="New browser tab" @click="emit('addBrowser')" />
          </Tooltip>
        </template>
      </TabStrip>
      <Tooltip content="Close panel" class="shrink-0">
        <IconButton :icon="PanelRightClose" variant="ghost" aria-label="Close panel" @click="emit('close')" />
      </Tooltip>
    </div>
    <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <slot :tab="active">
        <HostBrowserPanel
          v-if="active?.kind === 'host' && live"
          :session="live"
          :tab="hostTab"
        />
        <BrowserPanel v-else-if="active?.kind === 'browser'" :tab="active" @update="emit('updateBrowser', $event)" />
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
          :disabled="!menuId || tabsToClose(browserTabs, menuId, item.scope).length === 0"
          @select="closeScope(item.scope)"
        />
      </Menu>
    </Popover>
  </aside>
</template>

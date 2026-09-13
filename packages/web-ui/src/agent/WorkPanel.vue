<script setup lang="ts">
import { computed, ref } from 'vue'
import { Copy, File, FileDiff, PanelRightClose, Plus, X } from '@lucide/vue'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import { reportError } from '../infra/errors'
import { showToast } from '../infra/toast'
import { t } from '../infra/i18n'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from '../files/FileIcon.vue'
import ChangeView from '../files/ChangeView.vue'
import FileView from '../files/FileView.vue'
import { changeModeToOpen, type ChangeMode, type ChangeSources } from '../files/changes'
import { joinPath, normalizePath } from '../files/paths'
import type { FileBrowserSource } from '../files/types'
import TabItem from './TabItem.vue'
import TabStrip from './TabStrip.vue'
import { tabsToClose, type TabCloseScope } from './tab-close'
import { findChangeWorkTab, workTabTitle, type ChangeWorkTab, type WorkTab } from './work-panel'

/**
 * The work panel: the app frame's right pane, where the reader keeps files
 * and the conversation's changes (its diff) open beside it. It continues the
 * session's raised sheet behind a hairline divider, with the tab row (on the
 * raised surface) at the height of the session header. The host owns the
 * tabs and which one is active; the panel shows them, asks for new ones, and
 * says which to close.
 *
 * A tab's menu closes it, the others, or one side of it; a file tab also
 * copies its path. There is one change tab at most: asking for it again
 * selects the open one.
 *
 * A file tab shows its file through `workspace`: the source it reads from
 * and the root its paths are relative to. The file view's tree shows the
 * workspace; a file chosen there, or in a crumb's menu, is asked for with
 * `open`, and the host shows it in the active tab in place (see
 * `showFileInTab`), so Back and Forward walk that tab's files. The change
 * tab shows the workspace's changes when the host gives them, in the mode
 * and on the file the tab holds; a mode switch or a pick in its tree is
 * asked for with `showChange`, and the host steps the tab (see
 * `showChangeInTab`), so Back and Forward walk those steps. A new change tab
 * opens on what was picked from the conversation, if anything, else on the
 * uncommitted changes. Whether the trees show is one choice for the panel,
 * not per tab. Without a workspace the content pane is a placeholder.
 */
const props = defineProps<{
  tabs: readonly WorkTab[]
  activeId: string | null
  /** `name` stands in for the root directory's name wherever the views name the workspace. */
  workspace?: { source: FileBrowserSource; root: string; name?: string; changes?: ChangeSources }
}>()
const emit = defineEmits<{
  select: [id: string]
  closeTabs: [ids: string[]]
  /** New tab: a file, the host decides which; or the change tab, in the given mode. */
  add: [kind: 'file'] | [kind: 'change', mode: ChangeMode]
  /** The change tab's next step: this mode, showing this file (null leaves the choice to the view). */
  showChange: [id: string, mode: ChangeMode, path: string | null]
  /** A file from the tree or a crumb menu, by its path relative to the workspace root: show it in the active tab. */
  open: [path: string]
  /** The active tab's Back and Forward. */
  back: [id: string]
  forward: [id: string]
  /** The fold control: put the whole panel away. */
  close: []
}>()

const treeOpen = ref(true)

/** The file the change tab shows in its mode: the one it holds, else the first there is. */
function changeSelection(tab: ChangeWorkTab): string | null {
  const held = tab.selected[tab.mode]
  const files = props.workspace?.changes?.[tab.mode].files ?? []
  if (held !== null && files.some((file) => file.path === held)) {
    return held
  }
  return files[0]?.path ?? null
}

function absolutePath(path: string): string {
  return joinPath(props.workspace?.root ?? '/', path)
}

function openFromTree(path: string): void {
  const root = normalizePath(props.workspace?.root ?? '/')
  const relative = path.startsWith(`${root}/`) ? path.slice(root.length + 1) : path
  emit('open', relative)
}
const active = computed(
  () => props.tabs.find((tab) => tab.id === props.activeId) ?? null,
)

const addOpen = ref(false)

const menuTabId = ref<string | null>(null)
const menuTab = computed(
  () => props.tabs.find((tab) => tab.id === menuTabId.value) ?? null,
)
const {
  isOpen: menuOpen,
  anchorX: menuX,
  anchorY: menuY,
  anchorContextEl: menuEl,
  open: openMenu,
  close: closeMenu,
} = useContextMenuOwner(() => {
  menuTabId.value = null
})

function openTabMenu(event: MouseEvent, id: string): void {
  menuTabId.value = id
  openMenu(event)
}

function closeScope(scope: TabCloseScope): void {
  if (menuTabId.value === null) {
    return
  }
  const ids = tabsToClose(props.tabs, menuTabId.value, scope)
  if (ids.length > 0) {
    emit('closeTabs', ids)
  }
}

function scopeIsEmpty(scope: TabCloseScope): boolean {
  return menuTabId.value === null || tabsToClose(props.tabs, menuTabId.value, scope).length === 0
}

async function copyPath(): Promise<void> {
  const tab = menuTab.value
  if (tab?.kind !== 'file') {
    return
  }
  try {
    await navigator.clipboard.writeText(tab.path)
    showToast({ title: t('common.copied') })
  } catch (error) {
    reportError('Could not copy the path', error, { userVisible: true })
  }
}

function add(kind: WorkTab['kind']): void {
  addOpen.value = false
  if (kind === 'file') {
    emit('add', 'file')
    return
  }
  const open = findChangeWorkTab(props.tabs)
  if (open) {
    emit('select', open.id)
    return
  }
  const mode = props.workspace?.changes ? changeModeToOpen(props.workspace.changes) : 'uncommitted'
  emit('add', 'change', mode)
}
</script>

<template>
  <aside class="flex h-full min-w-0 flex-col overflow-hidden border-l border-line bg-surface text-fg">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
      <TabStrip class="flex-1" surface="raised">
        <TabItem
          v-for="tab in tabs"
          :key="tab.id"
          :tab="{ id: tab.id, title: workTabTitle(tab) }"
          :tooltip="tab.kind === 'file' ? tab.path : undefined"
          :is-active="tab.id === activeId"
          @pointerdown="emit('select', tab.id)"
          @contextmenu="openTabMenu($event, tab.id)"
          @close="emit('closeTabs', [tab.id])"
        >
          <template #mark>
            <FileIcon
              v-if="tab.kind === 'file'"
              :name="workTabTitle(tab)"
              :is-directory="false"
              :size="ICON_PX.markIn28"
            />
            <FileDiff
              v-else
              :size="ICON_PX.markIn28"
              class="text-fg-subtle"
            />
          </template>
        </TabItem>
        <template #trailing>
          <Dropdown
            v-model:open="addOpen"
            :overlay-store="appOverlayStore"
            placement="bottom-start"
          >
            <template #trigger="{ isOpen }">
              <IconButton
                class="ml-1"
                :icon="Plus"
                size="sm"
                variant="ghost"
                aria-label="New tab"
                :pressed="isOpen"
              />
            </template>
            <template #content>
              <Menu>
                <MenuItem :icon="File" label="File" @select="add('file')" />
                <MenuItem :icon="FileDiff" label="Change" @select="add('change')" />
              </Menu>
            </template>
          </Dropdown>
        </template>
      </TabStrip>
      <IconButton
        :icon="PanelRightClose"
        size="sm"
        variant="ghost"
        aria-label="Close panel"
        @click="emit('close')"
      />
    </div>
    <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <slot :tab="active">
        <!-- One view across file tabs, so the tree keeps what it has unfolded. -->
        <FileView
          v-if="active?.kind === 'file' && workspace"
          v-model:tree="treeOpen"
          :source="workspace.source"
          :root="workspace.root"
          :root-name="workspace.name"
          :path="absolutePath(active.path)"
          :can-back="active.back.length > 0"
          :can-forward="active.forward.length > 0"
          @open="openFromTree"
          @back="emit('back', active.id)"
          @forward="emit('forward', active.id)"
        />
        <ChangeView
          v-else-if="active?.kind === 'change' && workspace?.changes"
          v-model:tree="treeOpen"
          :mode="active.mode"
          :selected="changeSelection(active)"
          :changes="workspace.changes"
          :root="workspace.root"
          :root-name="workspace.name"
          :can-back="active.back.length > 0"
          :can-forward="active.forward.length > 0"
          @update:mode="emit('showChange', active.id, $event, active.selected[$event])"
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
      :is-open="menuOpen"
      :anchor-x="menuX"
      :anchor-y="menuY"
      :anchor-context-el="menuEl"
      :offset="0"
      @close="closeMenu"
    >
      <Menu @click="closeMenu">
        <template v-if="menuTab?.kind === 'file'">
          <MenuItem :icon="Copy" label="Copy path" @select="copyPath" />
          <MenuDivider />
        </template>
        <!-- Only the plain close has an icon; the scoped closes would repeat it. -->
        <MenuItem :icon="X" label="Close" @select="closeScope('self')" />
        <MenuItem label="Close others" :disabled="scopeIsEmpty('others')" @select="closeScope('others')" />
        <MenuItem label="Close to the right" :disabled="scopeIsEmpty('right')" @select="closeScope('right')" />
        <MenuItem label="Close to the left" :disabled="scopeIsEmpty('left')" @select="closeScope('left')" />
      </Menu>
    </Popover>
  </aside>
</template>

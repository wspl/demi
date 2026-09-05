<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Blocks, FolderPlus, Settings, SquarePen, WandSparkles } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useContextMenuOwner } from '@demicodes/web-ui/composables/useContextMenuOwner'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Popover from '@demicodes/web-ui/ui/Popover.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { SidebarAccount, SidebarConversation, SidebarExtension, SidebarProject, SidebarReorder } from './types'
import { plainConversations, projectGroups } from './group-conversations'
import { useSidebarDrag } from './useSidebarDrag'
import { reorderPeers } from './reorder'
import { visibleEntries } from './list-model'
import { useSidebarList } from './useSidebarList'
import ExtensionFlyout from './ExtensionFlyout.vue'
import SidebarAccountRow from './SidebarAccount.vue'
import SidebarNavItem from './SidebarNavItem.vue'
import SidebarProjectHeader from './SidebarProjectHeader.vue'
import SidebarProjectMenu from './SidebarProjectMenu.vue'
import SidebarRow from './SidebarRow.vue'
import SidebarSelectionMenu from './SidebarSelectionMenu.vue'

/**
 * Top: the app and its entries (new, plugins, skills). Middle: plain conversations, then every
 * project as a collapsible group of its conversations, with one selection across all of them.
 * Bottom: the account and settings.
 */
const props = defineProps<{
  account: SidebarAccount
  projects: SidebarProject[]
  conversations: SidebarConversation[]
  activeId: string | null
  plugins: SidebarExtension[]
  skills: SidebarExtension[]
  /** Pinned open flyout for the catalog; the product never pins. */
  hideExtensions?: boolean
  hidePin?: boolean
  hideDelete?: boolean
  pinnedFlyout?: 'plugins' | 'skills'
}>()

const emit = defineEmits<{
  reorder: [request: SidebarReorder]
  select: [id: string]
  create: [projectId: string | null]
  addProject: []
  removeProject: [id: string]
  rename: [id: string, title: string]
  pin: [ids: string[], pinned: boolean]
  moveToProject: [ids: string[], projectId: string | null]
  archive: [ids: string[]]
  remove: [ids: string[]]
  togglePlugin: [id: string, enabled: boolean]
  toggleSkill: [id: string, enabled: boolean]
  openSettings: []
  signOut: []
}>()

const collapsedProjects = defineModel<string[]>('collapsedProjects', { default: () => [] })
const renamingId = ref<string | null>(null)
const listRef = ref<HTMLElement>()
const drag = useSidebarDrag(listRef, () => props.projects, () => props.conversations, (request) => emit('reorder', request))

const plain = computed(() => plainConversations(props.conversations))
const groups = computed(() => projectGroups(props.projects, props.conversations))
const foldedSet = computed(() => new Set(
  drag.source.value?.kind === 'project'
    ? props.projects.map((project) => project.id)
    : collapsedProjects.value,
))
const entries = computed(() => visibleEntries(plain.value, groups.value, foldedSet.value))
const byId = computed(() => new Map(props.conversations.map((conversation) => [conversation.id, conversation])))
const projectById = computed(() => new Map(props.projects.map((project) => [project.id, project])))
const displayEntries = computed(() => [
  { kind: 'heading' as const, id: 'conversations-heading' },
  ...entries.value.filter((entry) => entry.kind === 'conversation' && entry.projectId === null),
  { kind: 'heading' as const, id: 'projects-heading' },
  ...entries.value.filter((entry) => entry.kind === 'project' || entry.projectId !== null),
])
const enabledPlugins = computed(() => props.plugins.filter((plugin) => plugin.enabled).length)
const enabledSkills = computed(() => props.skills.filter((skill) => skill.enabled).length)

// Folding is transient; reveal the source again after layout has settled on drop or cancellation.
watch(drag.source, async (source, previous, onCleanup) => {
  const project = source?.kind === 'project' ? source : previous?.kind === 'project' ? previous : null
  if (!project) return
  let cancelled = false
  onCleanup(() => { cancelled = true })
  await nextTick()
  const container = listRef.value
  const row = Array.from(container?.querySelectorAll<HTMLElement>('[data-sidebar-kind="project"]') ?? [])
    .find((element) => element.dataset.sidebarId === project.id)
  if (!container || !row) return
  await Promise.allSettled(row.getAnimations().map((animation) => animation.finished))
  if (cancelled || !row.isConnected) return
  const bounds = container.getBoundingClientRect()
  const item = row.getBoundingClientRect()
  const delta = source
    ? item.top < bounds.top ? item.top - bounds.top
      : item.bottom > bounds.bottom ? item.bottom - bounds.bottom : 0
    : item.top + item.height / 2 - (bounds.top + bounds.height / 2)
  if (delta) container.scrollBy({
    top: delta,
    behavior: source || window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth',
  })
})

function isFolded(projectId: string): boolean {
  return foldedSet.value.has(projectId)
}

function setFolded(projectId: string, folded: boolean): void {
  if (folded === isFolded(projectId)) return
  collapsedProjects.value = folded
    ? [...collapsedProjects.value, projectId]
    : collapsedProjects.value.filter((id) => id !== projectId)
}

function conversationsOf(ids: readonly string[]): SidebarConversation[] {
  return ids.flatMap((id) => {
    const conversation = byId.value.get(id)
    return conversation ? [conversation] : []
  })
}

function togglePin(ids: string[]): void {
  if (props.hidePin) return
  const targets = conversationsOf(ids)
  emit('pin', ids, !targets.every((target) => target.pinned))
}

const list = useSidebarList(entries, computed(() => props.activeId), {
  open: (id) => emit('select', id),
  toggleFold: (projectId) => setFolded(projectId, !isFolded(projectId)),
  fold: setFolded,
  rename: (id) => { renamingId.value = id },
  remove: (ids) => { if (!props.hideDelete) emit('remove', ids) },
  togglePin,
})

// The open conversation is the selection until the user makes a wider one.
watch(() => props.activeId, (id) => {
  if (id && !list.isSelected(id)) list.selectOnly(id)
}, { immediate: true })
watch(entries, list.prune)

// One context menu for rows and one for project headers, anchored where the pointer opened them.
const rowMenu = useContextMenuOwner()
const projectMenu = useContextMenuOwner()
const menuTargets = ref<SidebarConversation[]>([])
const menuProject = ref<SidebarProject | null>(null)

function openRowMenu(id: string, event: MouseEvent): void {
  list.onRowContextMenu(id)
  menuTargets.value = conversationsOf(list.targetIds(id))
  rowMenu.open(event, listRef.value)
}

function openProjectMenu(project: SidebarProject, event: MouseEvent): void {
  list.focusedId.value = project.id
  menuProject.value = project
  projectMenu.open(event, listRef.value)
}

// A key closes an open menu first: its targets are the selection the key is about to change.
function onListKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape' && drag.source.value) {
    event.preventDefault()
    drag.cancel()
    return
  }
  if (event.altKey && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
    const entry = entries.value.find((item) => item.id === list.focusedId.value)
    if (!entry) return
    event.preventDefault()
    const peers = reorderPeers(entry, props.projects, props.conversations)
    const index = peers.indexOf(entry.id)
    if (event.key === 'ArrowUp' && index > 0)
      emit('reorder', { kind: entry.kind, id: entry.id, beforeId: peers[index - 1]! })
    if (event.key === 'ArrowDown' && index >= 0 && index < peers.length - 1)
      emit('reorder', { kind: entry.kind, id: entry.id, beforeId: peers[index + 2] ?? null })
    return
  }
  if (rowMenu.isOpen.value || projectMenu.isOpen.value) {
    rowMenu.close()
    projectMenu.close()
    if (event.key === 'Escape') {
      event.preventDefault()
      return
    }
  }
  list.onKeydown(event)
}

function rowMenuOpenFor(id: string): boolean {
  return rowMenu.isOpen.value && menuTargets.value.some((target) => target.id === id)
}

function submitRename(id: string, title: string): void {
  renamingId.value = null
  const trimmed = title.trim()
  if (trimmed) emit('rename', id, trimmed)
  listRef.value?.focus()
}

function selectProjectConversations(project: SidebarProject): void {
  const group = groups.value.find((candidate) => candidate.project.id === project.id)
  if (!group || group.items.length === 0) return
  setFolded(project.id, false)
  list.selected.value = new Set(group.items.map((item) => item.id))
  list.focusedId.value = group.items[0]!.id
}
</script>

<template>
  <aside
    class="flex h-full shrink-0 select-none flex-col bg-surface-base text-fg w-64"
  >
    <div class="flex h-11 shrink-0 items-center px-2.5">
      <span class="min-w-0 flex-1 truncate text-chrome font-medium text-fg-emphasis">Demi</span>
    </div>

    <!-- The entries: one primary action, and what the agent can use. -->
    <div class="flex shrink-0 flex-col gap-px px-2.5">
      <SidebarNavItem :icon="SquarePen" label="New conversation" shortcut="⌘N" emphasis @click="emit('create', null)" />
      <Dropdown v-if="!hideExtensions" :overlay-store="appOverlayStore" placement="bottom-start" :offset="8" v-bind="pinnedFlyout === 'plugins' ? { open: true } : {}">
        <template #trigger="{ isOpen }">
          <SidebarNavItem :icon="Blocks" label="Plugins" :count="enabledPlugins" :pressed="isOpen" />
        </template>
        <template #content="{ close }">
          <ExtensionFlyout
            title="Plugins"
            :items="plugins"
            manage-label="Manage plugins…"
            @toggle="(id, enabled) => emit('togglePlugin', id, enabled)"
            @manage="close"
          />
        </template>
      </Dropdown>
      <Dropdown v-if="!hideExtensions" :overlay-store="appOverlayStore" placement="bottom-start" :offset="8" v-bind="pinnedFlyout === 'skills' ? { open: true } : {}">
        <template #trigger="{ isOpen }">
          <SidebarNavItem :icon="WandSparkles" label="Skills" :count="enabledSkills" :pressed="isOpen" />
        </template>
        <template #content="{ close }">
          <ExtensionFlyout
            title="Skills"
            :items="skills"
            manage-label="Browse skills…"
            @toggle="(id, enabled) => emit('toggleSkill', id, enabled)"
            @manage="close"
          />
        </template>
      </Dropdown>
    </div>

    <slot name="navigation" />

    <!-- Plain conversations first, then the projects. One focusable list; rows are not tab stops. -->
    <div
      ref="listRef"
      class="mt-4 min-h-0 flex-1 overflow-y-auto sidebar-scroll px-2.5 pb-2 outline-none"
      :class="list.keyboardNav.value ? 'is-keyboard' : ''"
      tabindex="0"
      role="listbox"
      aria-multiselectable="true"
      @keydown="onListKeydown"
      @pointerdown="list.keyboardNav.value = false; drag.pointerDown()"
      @click.capture="drag.click"

    >
      <TransitionGroup name="sidebar-items" tag="div" class="relative">
        <div
          v-for="(entry, index) in displayEntries"
          :key="entry.id"
          class="sidebar-entry relative"
          :class="[
            entry.kind === 'project' && displayEntries[index - 1]?.id !== 'projects-heading' ? 'mt-3' : '',
            drag.source.value?.id === entry.id ? 'opacity-40' : '',
            drag.target.value?.id === entry.id ? (drag.target.value.after ? 'drop-after' : 'drop-before') : '',
          ]"
          :data-sidebar-id="entry.id"
          :data-sidebar-kind="entry.kind"
          @pointerdown="entry.kind !== 'heading' && renamingId !== entry.id && drag.start($event, entry)"
        >
          <div v-if="entry.kind === 'heading'" class="group/projects mb-1.5 flex h-6 items-center px-2 text-[11px] uppercase tracking-wide text-fg-subtle" :class="entry.id === 'projects-heading' ? 'mt-4' : ''">
            <span class="flex-1">{{ entry.id === 'projects-heading' ? 'Projects' : 'Conversations' }}</span>
            <Tooltip v-if="entry.id === 'projects-heading'" content="Add project" placement="right">
              <IconButton :icon="FolderPlus" size="xs" variant="ghost" class="opacity-0 transition-opacity group-hover/projects:opacity-100" @click="emit('addProject')" />
            </Tooltip>
          </div>
          <SidebarProjectHeader
            v-else-if="entry.kind === 'project'"
            :project="projectById.get(entry.id)!"
            :collapsed="isFolded(entry.id)"
            :focused="list.keyboardNav.value && list.focusedId.value === entry.id"
            :menu-open="projectMenu.isOpen.value && menuProject?.id === entry.id"
            @toggle="list.onProjectClick(entry.id)"
            @create="emit('create', entry.id)"
            @contextmenu="(event) => openProjectMenu(projectById.get(entry.id)!, event)"
          />
          <SidebarRow
            v-else
            :conversation="byId.get(entry.id)!"
            :hide-pin="hidePin"
            :open="entry.id === activeId"
            :selected="list.isSelected(entry.id)"
            :focused="list.keyboardNav.value && list.focusedId.value === entry.id"
            :menu-open="rowMenuOpenFor(entry.id)"
            :renaming="renamingId === entry.id"
            @click="(event) => list.onRowClick(entry.id, event)"
            @contextmenu="(event) => openRowMenu(entry.id, event)"
            @archive="emit('archive', [entry.id])"
            @rename-submit="(title) => submitRename(entry.id, title)"
            @rename-cancel="renamingId = null"
            @toggle-pin="togglePin([entry.id])"
          />
        </div>
      </TransitionGroup>
    </div>
    <Teleport to="body">
      <div v-if="drag.source.value" class="pointer-events-none fixed z-50 max-w-56 truncate rounded-md border border-line bg-surface-raised px-3 py-1 text-chrome text-fg shadow-md" :style="{ left: `${drag.pointer.value.x + 12}px`, top: `${drag.pointer.value.y + 12}px` }" aria-hidden="true">
        {{ drag.source.value.kind === 'project' ? projectById.get(drag.source.value.id)?.name : byId.get(drag.source.value.id)?.title }}
      </div>
    </Teleport>

    <!-- The account, and settings. -->
    <div class="flex shrink-0 items-center gap-1 border-t border-line px-2.5 py-2">
      <SidebarAccountRow :account="account" @open-settings="emit('openSettings')" @sign-out="emit('signOut')" />
      <Tooltip content="Settings" placement="right">
        <IconButton :icon="Settings" variant="ghost" @click="emit('openSettings')" />
      </Tooltip>
    </div>

    <Popover
      :key="rowMenu.menuKey.value"
      :overlay-store="appOverlayStore"
      :is-open="rowMenu.isOpen.value"
      :anchor-x="rowMenu.anchorX.value"
      :anchor-y="rowMenu.anchorY.value"
      :anchor-context-el="rowMenu.anchorContextEl.value"
      :offset="0"
      @close="rowMenu.close()"
    >
      <SidebarSelectionMenu
        :targets="menuTargets"
        :hide-pin="hidePin"
        :hide-delete="hideDelete"
        :projects="projects"
        @open="(id) => { rowMenu.close(); emit('select', id) }"
        @rename="(id) => { rowMenu.close(); renamingId = id }"
        @pin="(ids, pinned) => { rowMenu.close(); emit('pin', ids, pinned) }"
        @move-to="(ids, projectId) => { rowMenu.close(); emit('moveToProject', ids, projectId) }"
        @archive="(ids) => { rowMenu.close(); emit('archive', ids) }"
        @remove="(ids) => { rowMenu.close(); emit('remove', ids) }"
      />
    </Popover>
    <Popover
      :key="projectMenu.menuKey.value"
      :overlay-store="appOverlayStore"
      :is-open="projectMenu.isOpen.value && menuProject !== null"
      :anchor-x="projectMenu.anchorX.value"
      :anchor-y="projectMenu.anchorY.value"
      :anchor-context-el="projectMenu.anchorContextEl.value"
      :offset="0"
      @close="projectMenu.close()"
    >
      <SidebarProjectMenu
        v-if="menuProject"
        :project="menuProject"
        :folded="isFolded(menuProject.id)"
        :count="groups.find((group) => group.project.id === menuProject?.id)?.items.length ?? 0"
        @create="projectMenu.close(); emit('create', menuProject.id)"
        @toggle-fold="projectMenu.close(); setFolded(menuProject.id, !isFolded(menuProject.id))"
        @select-all="projectMenu.close(); selectProjectConversations(menuProject)"
        @remove="projectMenu.close(); emit('removeProject', menuProject.id)"
      />
    </Popover>
  </aside>
</template>

<style scoped>
.sidebar-scroll {
  scrollbar-color: var(--fg-faint) transparent;
  scrollbar-gutter: stable;
}

.sidebar-items-move,
.sidebar-items-enter-active,
.sidebar-items-leave-active {
  transition: transform 180ms ease, opacity 180ms ease;
}
.sidebar-items-enter-from,
.sidebar-items-leave-to { opacity: 0; }
.sidebar-items-leave-active { position: absolute; width: 100%; pointer-events: none; }
.drop-before::before,
.drop-after::after {
  content: '';
  position: absolute;
  left: 4px;
  right: 4px;
  height: 2px;
  background: var(--on-accent);
  pointer-events: none;
  z-index: 1;
}
.drop-before::before { top: 0; }
.drop-after::after { bottom: 0; }
@media (prefers-reduced-motion: reduce) {
  .sidebar-items-move,
  .sidebar-items-enter-active,
  .sidebar-items-leave-active { transition: none; }
}
</style>

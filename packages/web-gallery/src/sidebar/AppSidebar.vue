<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Blocks, FolderPlus, PanelLeftClose, PanelLeftOpen, Settings, SquarePen, WandSparkles } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useContextMenuOwner } from '@demicodes/web-ui/composables/useContextMenuOwner'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Popover from '@demicodes/web-ui/ui/Popover.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { SidebarAccount, SidebarConversation, SidebarExtension, SidebarProject } from './sidebar-data'
import { plainConversations, projectGroups } from './group-conversations'
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
 * Bottom: the account and settings. Collapsed, the same entries as an icon rail.
 */
const props = defineProps<{
  account: SidebarAccount
  projects: SidebarProject[]
  conversations: SidebarConversation[]
  activeId: string | null
  plugins: SidebarExtension[]
  skills: SidebarExtension[]
  /** Pinned open flyout for the catalog; the product never pins. */
  pinnedFlyout?: 'plugins' | 'skills'
}>()

const emit = defineEmits<{
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

const collapsed = defineModel<boolean>('collapsed', { default: false })
const collapsedProjects = defineModel<string[]>('collapsedProjects', { default: () => [] })
const renamingId = ref<string | null>(null)
const listRef = ref<HTMLElement>()

const plain = computed(() => plainConversations(props.conversations))
const groups = computed(() => projectGroups(props.projects, props.conversations))
const foldedSet = computed(() => new Set(collapsedProjects.value))
const entries = computed(() => visibleEntries(plain.value, groups.value, foldedSet.value))
const byId = computed(() => new Map(props.conversations.map((conversation) => [conversation.id, conversation])))
const enabledPlugins = computed(() => props.plugins.filter((plugin) => plugin.enabled).length)
const enabledSkills = computed(() => props.skills.filter((skill) => skill.enabled).length)

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
  const targets = conversationsOf(ids)
  emit('pin', ids, !targets.every((target) => target.pinned))
}

const list = useSidebarList(entries, computed(() => props.activeId), {
  open: (id) => emit('select', id),
  toggleFold: (projectId) => setFolded(projectId, !isFolded(projectId)),
  fold: setFolded,
  rename: (id) => { renamingId.value = id },
  remove: (ids) => emit('remove', ids),
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
    class="flex h-full shrink-0 flex-col bg-surface-base text-fg transition-[width] duration-200 ease-out"
    :class="collapsed ? 'w-12' : 'w-64'"
  >
    <!-- The app, and the fold. -->
    <div class="flex h-11 shrink-0 items-center px-2.5" :class="collapsed ? 'justify-center' : 'gap-2'">
      <span class="flex size-7 shrink-0 items-center justify-center rounded-md bg-fg text-[13px] font-semibold text-surface-base">d</span>
      <template v-if="!collapsed">
        <span class="min-w-0 flex-1 truncate text-chrome font-medium text-fg-emphasis">Demi</span>
        <Tooltip content="Collapse sidebar" placement="right">
          <IconButton :icon="PanelLeftClose" variant="ghost" @click="collapsed = true" />
        </Tooltip>
      </template>
    </div>

    <!-- The entries: one primary action, and what the agent can use. -->
    <div class="flex shrink-0 flex-col gap-px px-2.5" :class="collapsed ? 'items-center' : ''">
      <SidebarNavItem :icon="SquarePen" label="New conversation" shortcut="⌘N" :collapsed="collapsed" emphasis @click="emit('create', null)" />
      <Dropdown :overlay-store="appOverlayStore" placement="bottom-start" :offset="8" v-bind="pinnedFlyout === 'plugins' ? { open: true } : {}">
        <template #trigger="{ isOpen }">
          <SidebarNavItem :icon="Blocks" label="Plugins" :count="collapsed ? undefined : enabledPlugins" :collapsed="collapsed" :pressed="isOpen" />
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
      <Dropdown :overlay-store="appOverlayStore" placement="bottom-start" :offset="8" v-bind="pinnedFlyout === 'skills' ? { open: true } : {}">
        <template #trigger="{ isOpen }">
          <SidebarNavItem :icon="WandSparkles" label="Skills" :count="collapsed ? undefined : enabledSkills" :collapsed="collapsed" :pressed="isOpen" />
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

    <!-- Plain conversations first, then the projects. One focusable list; rows are not tab stops. -->
    <div
      v-if="!collapsed"
      ref="listRef"
      class="mt-4 min-h-0 flex-1 overflow-y-auto scrollbar-hidden px-2.5 pb-2 outline-none"
      :class="list.keyboardNav.value ? 'is-keyboard' : ''"
      tabindex="0"
      role="listbox"
      aria-multiselectable="true"
      @keydown="onListKeydown"
      @pointerdown="list.keyboardNav.value = false"
    >
      <section class="mb-4">
        <div class="flex h-6 items-center px-2 text-[11px] uppercase tracking-wide text-fg-subtle">Conversations</div>
        <div v-if="plain.length === 0" class="px-2 py-1 text-[12px] leading-5 text-fg-faint">Nothing outside a project yet.</div>
        <div v-else class="flex flex-col gap-px">
          <SidebarRow
            v-for="conversation in plain"
            :key="conversation.id"
            :conversation="conversation"
            :open="conversation.id === activeId"
            :selected="list.isSelected(conversation.id)"
            :focused="list.keyboardNav.value && list.focusedId.value === conversation.id"
            :menu-open="rowMenuOpenFor(conversation.id)"
            :renaming="renamingId === conversation.id"
            @click="(event) => list.onRowClick(conversation.id, event)"
            @contextmenu="(event) => openRowMenu(conversation.id, event)"
            @menu="(event) => openRowMenu(conversation.id, event)"
            @rename-submit="(title) => submitRename(conversation.id, title)"
            @rename-cancel="renamingId = null"
            @toggle-pin="togglePin([conversation.id])"
          />
        </div>
      </section>

      <section>
        <div class="group/projects flex h-6 items-center px-2 text-[11px] uppercase tracking-wide text-fg-subtle">
          <span class="flex-1">Projects</span>
          <Tooltip content="Add project" placement="right">
            <IconButton :icon="FolderPlus" size="xs" variant="ghost" class="opacity-0 transition-opacity group-hover/projects:opacity-100" @click="emit('addProject')" />
          </Tooltip>
        </div>
        <div v-if="groups.length === 0" class="px-2 py-1 text-[12px] leading-5 text-fg-faint">Open a folder to start a project.</div>
        <div v-for="group in groups" :key="group.project.id" class="mb-3 last:mb-0">
          <SidebarProjectHeader
            :project="group.project"
            :collapsed="isFolded(group.project.id)"
            :focused="list.keyboardNav.value && list.focusedId.value === group.project.id"
            :menu-open="projectMenu.isOpen.value && menuProject?.id === group.project.id"
            @toggle="list.onProjectClick(group.project.id)"
            @contextmenu="(event) => openProjectMenu(group.project, event)"
          />
          <!-- The fold animates on grid rows, so the rows stay mounted and their state survives. -->
          <div class="sidebar-fold" :class="isFolded(group.project.id) ? '' : 'is-open'" :aria-hidden="isFolded(group.project.id) || undefined">
          <div class="overflow-hidden">
          <div class="flex flex-col gap-px">
            <SidebarRow
              v-for="conversation in group.items"
              :key="conversation.id"
              :conversation="conversation"
              :open="conversation.id === activeId"
              :selected="list.isSelected(conversation.id)"
              :focused="list.keyboardNav.value && list.focusedId.value === conversation.id"
              :menu-open="rowMenuOpenFor(conversation.id)"
              :renaming="renamingId === conversation.id"
              @click="(event) => list.onRowClick(conversation.id, event)"
              @contextmenu="(event) => openRowMenu(conversation.id, event)"
              @menu="(event) => openRowMenu(conversation.id, event)"
              @rename-submit="(title) => submitRename(conversation.id, title)"
              @rename-cancel="renamingId = null"
              @toggle-pin="togglePin([conversation.id])"
            />
            <div
              v-if="group.items.length === 0"
              role="button"
              class="flex h-7 cursor-default select-none items-center gap-2 rounded-md px-2 text-chrome text-fg-subtle transition-colors hover:bg-hover hover:text-fg-muted"
              @click="emit('create', group.project.id)"
            >
              <SquarePen :size="ICON_PX.in28" class="shrink-0" />
              <span class="truncate">New conversation here</span>
            </div>
          </div>
          </div>
          </div>
        </div>
      </section>
    </div>
    <div v-else class="flex-1" />

    <!-- The account, and settings. -->
    <div class="flex shrink-0 items-center gap-1 border-t border-line px-2.5 py-2" :class="collapsed ? 'flex-col' : ''">
      <SidebarAccountRow :account="account" :collapsed="collapsed" @open-settings="emit('openSettings')" @sign-out="emit('signOut')" />
      <Tooltip content="Settings" placement="right">
        <IconButton :icon="Settings" variant="ghost" @click="emit('openSettings')" />
      </Tooltip>
      <Tooltip v-if="collapsed" content="Expand sidebar" placement="right">
        <IconButton :icon="PanelLeftOpen" variant="ghost" @click="collapsed = false" />
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
.sidebar-fold {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 0.2s ease;
}

.sidebar-fold.is-open {
  grid-template-rows: 1fr;
}
</style>

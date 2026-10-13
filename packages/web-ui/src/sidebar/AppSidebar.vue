<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Archive, FolderPlus, Search, Settings, Smartphone, SquarePen } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useContextMenuOwner } from '@demicodes/web-ui/composables/useContextMenuOwner'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import ContextMenu from '@demicodes/web-ui/ui/ContextMenu.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import RegionStatus from '@demicodes/web-ui/ui/RegionStatus.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import { FOLD_MS } from '@demicodes/web-ui/ui/fold'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import { copyConversationId } from '../agent/copy-conversation-id'
import type {
  ListLoad,
  SidebarAccount,
  SidebarConversation,
  SidebarEntry,
  SidebarProject,
  SidebarReorder
} from './types'
import { plainConversations, projectGroups } from './group-conversations'
import { useSidebarDrag } from './useSidebarDrag'
import { reorderPeers } from './reorder'
import { labelWaiting, listLines, type SidebarGroup } from './list-model'
import { useSidebarList } from './useSidebarList'
import SidebarAccountRow from './SidebarAccount.vue'
import SidebarGroupLabel from './SidebarGroupLabel.vue'
import SidebarNavItem from './SidebarNavItem.vue'
import SidebarProjectHeader from './SidebarProjectHeader.vue'
import { projectsNamingTheirHost } from './project-hosts'
import SidebarProjectMenu from './SidebarProjectMenu.vue'
import SidebarRow from './SidebarRow.vue'
import SidebarSelectionMenu from './SidebarSelectionMenu.vue'
import { useRevealOpen } from './reveal-open'
import ConversationDeleteDialog from './ConversationDeleteDialog.vue'

/**
 * Top: the app and its entries: New, Search, then the entries the host gives, such as a
 * plugin's settings section, then Archived; each but New and Search opens its settings section. Middle: plain
 * conversations, then every project as a collapsible group of its conversations, with one
 * selection across all of them; the Conversations and Projects groups each fold under their label.
 * Bottom: the account and settings.
 */
const props = defineProps<{
  account: SidebarAccount
  projects: SidebarProject[]
  conversations: SidebarConversation[]
  activeId: string | null
  /** `loading` is a spinner, not a first-run empty list. */
  listStatus?: ListLoad
  /** The entries between New and Archived, each opening its settings section. */
  sectionEntries?: readonly SidebarEntry[]
  /** The keys New conversation has in the keyboard settings, which New shows; none when it has none. */
  newShortcut?: string
  /** The keys Search conversations has in the keyboard settings, which Search shows; none when it has none. */
  searchShortcut?: string
}>()

const emit = defineEmits<{
  reorder: [request: SidebarReorder]
  select: [id: string]
  create: [projectId: string | null]
  /** Opens the search window. */
  search: []
  addProject: []
  removeProject: [id: string]
  rename: [id: string, title: string]
  pin: [ids: string[], pinned: boolean]
  archive: [ids: string[]]
  /** Delete, once its dialog was answered: the conversations go for good. */
  delete: [ids: string[]]
  /** Settings, on a section when an entry names one (`archived`, or an entry's own). */
  openSettings: [section?: string]
  /** The phone mark at the foot: sign in on a phone by a QR code. */
  signInOnPhone: []
  signOut: []
  retryList: []
}>()

/** On a phone the list's rows are a finger's height (`ui/phone.ts`). */
const phone = usePhone()

const collapsedProjects = defineModel<string[]>('collapsedProjects', { default: () => [] })
/** The groups folded under their label; the host keeps them per browser. */
const foldedGroups = defineModel<SidebarGroup[]>('foldedGroups', { default: () => [] })
const renamingId = ref<string | null>(null)
const listRef = ref<HTMLElement>()
const scrollArea = ref<InstanceType<typeof ScrollArea>>()
/** The element the list scrolls in. */
const scroller = computed(() => scrollArea.value?.el)
const drag = useSidebarDrag(
  scroller,
  () => props.projects,
  () => props.conversations,
  request => emit('reorder', request),
)

const plain = computed(() => plainConversations(props.conversations))
const groups = computed(() => projectGroups(props.projects, props.conversations))
const foldedSet = computed(() => {
  // Hide all project children while their headers are being reordered.
  if (drag.source.value?.kind === 'project') {
    return new Set(props.projects.map(project => project.id))
  }
  return new Set(collapsedProjects.value)
})
const lines = computed(() => listLines(
  plain.value,
  groups.value,
  foldedSet.value,
  new Set(foldedGroups.value),
  props.activeId,
))
/** The lines the keyboard reaches: those no folded group hides. */
const entries = computed(() => lines.value.filter((line) => !line.hidden))
const byId = computed(
  () => new Map(
    props.conversations.map((conversation) => [conversation.id, conversation])
  )
)
const projectById = computed(() => new Map(props.projects.map((project) => [project.id, project])))
/** The projects whose row names its host, since a project of the same name runs on another. */
const namesakes = computed(() => projectsNamingTheirHost(props.projects))
/** Each group's label, in list order, with the dot it carries while folded. */
const groupLabels = computed(() => {
  const label = (group: SidebarGroup, text: string, conversations: readonly SidebarConversation[]) => {
    const folded = foldedGroups.value.includes(group)
    return {
      kind: 'heading' as const,
      id: `${group}-heading`,
      // A label is never hidden; its group's lines are.
      hidden: false,
      group,
      text,
      folded,
      waiting: labelWaiting(folded, conversations, props.activeId),
    }
  }
  return {
    conversations: label('conversations', 'Conversations', plain.value),
    projects: label('projects', 'Projects', groups.value.flatMap((group) => group.items)),
  }
})
const displayEntries = computed(() => [
  groupLabels.value.conversations,
  ...lines.value.filter(
    (line) => line.kind === 'conversation' && line.projectId === null
  ),
  groupLabels.value.projects,
  ...lines.value.filter(
    (line) => line.kind === 'project' || line.projectId !== null
  ),
])

/** The first project header shown, which sits right under the Projects label without a gap. */
const firstShownProject = computed(() => lines.value.find((line) => line.kind === 'project' && !line.hidden)?.id)

function toggleGroup(group: SidebarGroup): void {
  foldedGroups.value = foldedGroups.value.includes(group)
    ? foldedGroups.value.filter((each) => each !== group)
    : [...foldedGroups.value, group]
}

// Folding is transient; reveal the source again after layout has settled on drop or cancellation.
watch(drag.source, async (source, previous, onCleanup) => {
  const project = source ?? previous
  if (project?.kind !== 'project')
    return

  let cancelled = false
  onCleanup(() => {
    cancelled = true
  })

  await nextTick()
  if (cancelled)
    return

  const container = scroller.value
  if (!container)
    return
  const rows = container.querySelectorAll<HTMLElement>('[data-sidebar-kind="project"]')
  const row = Array.from(rows).find(element => element.dataset.sidebarId === project.id)
  if (!row)
    return

  // Cancelled transitions still leave a measurable row; only the watcher cancellation stops scrolling.
  await Promise.allSettled(row.getAnimations().map(animation => animation.finished))
  if (cancelled || !row.isConnected)
    return

  const bounds = container.getBoundingClientRect()
  const item = row.getBoundingClientRect()
  let distance = 0
  let behavior: ScrollBehavior = 'instant'

  if (source) {
    // Keep the dragged header visible without shifting it unnecessarily.
    if (item.top < bounds.top) {
      distance = item.top - bounds.top
    } else if (item.bottom > bounds.bottom) {
      distance = item.bottom - bounds.bottom
    }
  } else {
    // Once the children unfold, bring their project header back to the center.
    distance = item.top + item.height / 2 - (bounds.top + bounds.height / 2)
    if (!window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      behavior = 'smooth'
    }
  }

  if (distance) {
    container.scrollBy({ top: distance, behavior })
  }
})

function isFolded(projectId: string): boolean {
  return foldedSet.value.has(projectId)
}

function setFolded(projectId: string, folded: boolean): void {
  if (folded === isFolded(projectId))
    return
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
  rename: (id) => {
    renamingId.value = id
  },
  togglePin,
})

/**
 * Scrolls only the list, not the page around the sidebar. TypeScript's DOM
 * types do not list `container` yet (CSSOM View's ScrollIntoViewOptions).
 */
const SCROLL_IN_LIST: ScrollIntoViewOptions & { container: 'nearest' } = {
  block: 'nearest',
  container: 'nearest',
}

/**
 * The entry keyboard focus moves to, or that a key reorders, scrolls into
 * view, clear of the headings that stick above it: the list's heading (the
 * viewport's scroll padding) and, for a project's row, its project header
 * (the row's scroll margin). It scrolls once the update and the entry's
 * glide are done: the TransitionGroup compares each row's place before and
 * after an update, and a scroll in between would read as every row moving.
 */
watch(
  () => [list.focusedId.value, list.keyboardNav.value, entries.value] as const,
  async ([id, keyboard], _previous, onCleanup) => {
    if (!keyboard || id === null)
      return
    let cancelled = false
    onCleanup(() => {
      cancelled = true
    })
    await nextTick()
    const entry = scroller.value?.querySelector<HTMLElement>(`[data-sidebar-id="${CSS.escape(id)}"]`)
    if (!entry)
      return
    // A reordered entry glides from its old place first; it scrolls into view where it lands.
    // A cancelled glide still lands, and the next key's scroll supersedes this one.
    await Promise.allSettled(entry.getAnimations().map((animation) => animation.finished))
    if (cancelled || !entry.isConnected)
      return
    entry.scrollIntoView(SCROLL_IN_LIST)
  },
)

useRevealOpen({
  openProject: () => (props.activeId ? byId.value.get(props.activeId)?.projectId ?? null : null),
  isFolded,
  unfold: (projectId) => setFolded(projectId, false),
})

// The open conversation is the selection until the user makes a wider one.
watch(() => props.activeId, (id) => {
  if (id && !list.isSelected(id))
    list.selectOnly(id)
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

function onListPointerDown() {
  list.keyboardNav.value = false
  drag.pointerDown()
}

// A key closes an open menu first: its targets are the selection the key is about to change.
// A key typed in a control inside the list, a label's fold or a row's action, is the control's:
// Enter on a focused label folds its group rather than opening the focused row.
function onListKeydown(event: KeyboardEvent): void {
  if (event.target !== event.currentTarget)
    return
  if (event.key === 'Escape' && drag.source.value) {
    event.preventDefault()
    drag.cancel()
    return
  }
  if (event.altKey && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
    const entry = entries.value.find((item) => item.id === list.focusedId.value)
    if (!entry)
      return
    event.preventDefault()
    const peers = reorderPeers(entry, props.projects, props.conversations)
    const index = peers.indexOf(entry.id)
    if (event.key === 'ArrowUp' && index > 0) {
      emit('reorder', {
        kind: entry.kind,
        id: entry.id,
        beforeId: peers[index - 1]!
      })
    }
    if (event.key === 'ArrowDown' && index >= 0 && index < peers.length - 1) {
      emit(
        'reorder',
        {
          kind: entry.kind,
          id: entry.id,
          beforeId: peers[index + 2] ?? null
        }
      )
    }
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

/* Rows grow and shrink in flow. A leaving row keeps its place and folds its height to
   nothing while it fades, so the list's height, the scroll position and the rows below all
   follow it smoothly; a row taken out of flow would vanish in one step at the end, and the
   scroll position with it. The gap and a project's top margin fold with the row. */
function beforeEnterEntry(el: Element): void {
  const row = el as HTMLElement
  row.style.height = '0'
  row.style.overflow = 'hidden'
}
function enterEntry(el: Element): void {
  const row = el as HTMLElement
  row.style.height = `${row.scrollHeight}px`
}
function afterEnterEntry(el: Element): void {
  const row = el as HTMLElement
  row.style.height = ''
  row.style.overflow = ''
}
function beforeLeaveEntry(el: Element): void {
  const row = el as HTMLElement
  row.style.height = `${row.offsetHeight}px`
  row.style.overflow = 'hidden'
}
function leaveEntry(el: Element): void {
  const row = el as HTMLElement
  row.style.height = '0'
  row.style.marginTop = '0'
  row.style.marginBottom = '-1px'
}

// Delete asks first. The dialog keeps its targets while it closes, so its leave shows what it asked.
const deleteTargets = ref<SidebarConversation[]>([])
const deleteOpen = ref(false)

function askDelete(ids: string[]): void {
  deleteTargets.value = conversationsOf(ids)
  deleteOpen.value = deleteTargets.value.length > 0
}

function confirmDelete(): void {
  deleteOpen.value = false
  emit('delete', deleteTargets.value.map((target) => target.id))
}

function rowMenuOpenFor(id: string): boolean {
  return rowMenu.isOpen.value && menuTargets.value.some((target) => target.id === id)
}

function submitRename(id: string, title: string): void {
  renamingId.value = null
  emit('rename', id, title)
  listRef.value?.focus()
}

// A cancelled rename gives the keys back to the list too, as a submitted one does.
function cancelRename(): void {
  renamingId.value = null
  listRef.value?.focus()
}

function selectProjectConversations(project: SidebarProject): void {
  const group = groups.value.find((candidate) => candidate.project.id === project.id)
  if (!group || group.items.length === 0)
    return
  setFolded(project.id, false)
  list.selected.value = new Set(group.items.map((item) => item.id))
  list.focusedId.value = group.items[0]!.id
}
</script>

<template>
  <aside
    class="flex h-full w-[var(--sidebar-width,16rem)] shrink-0 select-none flex-col bg-surface-base text-fg"
  >
    <div class="flex h-11 shrink-0 items-center px-2">
      <span
        class="min-w-0 flex-1 truncate pl-2.5 text-chrome font-medium text-fg-emphasis phone:pl-3 phone:text-[17px] phone:font-semibold"
      >Demi</span>
    </div>

    <!-- Every block of the sidebar keeps 8px from both edges, so a row's background ends the same
         distance from the left edge as from the right. The list scrolls in a ScrollArea, whose
         scrollbar takes no room, so the inset is the same whether the system shows scrollbars or
         overlays them, and the list scrolls without shifting anything. A row's content
         starts 10px inside that, so the title, entries, headings, project headers and conversation
         rows align at one line while a lit row has room around its status dot. Row actions sit 2px
         inside the row's end, the same as their 2px above and below in the 28px row. -->
    <!-- The entries, all alike: New, Search, then the settings sections a conversation reaches for. -->
    <div class="flex shrink-0 flex-col gap-px px-2">
      <SidebarNavItem
        :icon="SquarePen"
        label="New"
        :shortcut="newShortcut"
        @click="emit('create', null)"
      />
      <SidebarNavItem
        :icon="Search"
        label="Search"
        :shortcut="searchShortcut"
        @click="emit('search')"
      />
      <SidebarNavItem
        v-for="entry in sectionEntries"
        :key="entry.section"
        :icon="entry.icon"
        :label="entry.label"
        @click="emit('openSettings', entry.section)"
      />
      <SidebarNavItem
        :icon="Archive"
        label="Archived"
        @click="emit('openSettings', 'archived')"
      />
    </div>

    <!-- Plain conversations first, then the projects. One focusable list; rows are not tab stops. -->
    <ScrollArea
      ref="scrollArea"
      class="mt-4 min-h-0 flex-1"
      viewport-class="flex scroll-pt-[30px] flex-col px-2 phone:scroll-pt-[42px]"
    >
      <div
        ref="listRef"
        class="flex-1 pb-2 outline-none"
        :class="list.keyboardNav.value ? 'is-keyboard' : ''"
        tabindex="0"
        role="listbox"
        aria-multiselectable="true"
        :aria-busy="listStatus === 'loading' || undefined"
        @keydown="onListKeydown"
        @pointerdown="onListPointerDown"
        @click.capture="drag.click"

      >
        <RegionStatus
          v-if="listStatus === 'loading' || listStatus === 'failed'"
          class="h-full min-h-40"
          :status="listStatus"
          :label="listStatus === 'loading' ? 'Loading conversations…' : 'Couldn’t load conversations.'"
          loading-label="Loading conversations…"
          :on-retry="() => emit('retryList')"
        />
        <!-- Rows sit a hairline apart, the way menu items and the entries above do. -->
        <TransitionGroup
          v-else
          name="sidebar-items"
          tag="div"
          class="relative flex flex-col gap-px"
          :style="{ '--fold-ms': `${FOLD_MS}ms` }"
          @before-enter="beforeEnterEntry"
          @enter="enterEntry"
          @after-enter="afterEnterEntry"
          @enter-cancelled="afterEnterEntry"
          @before-leave="beforeLeaveEntry"
          @leave="leaveEntry"
        >
          <div
            v-for="entry in displayEntries"
            :key="entry.id"
            class="sidebar-entry relative"
            :class="[
              // A line its folded group hides gives back the 1px gap above it, so the label stands alone.
              entry.hidden ? '-mt-px' : entry.kind === 'project' && entry.id !== firstShownProject ? 'mt-3' : '',
              // Headings stick to the top of the list; a project's header sticks just under the Projects heading,
              // 30px, and 42px on a phone, where rows are a finger's height.
              // The gap above Projects is a margin on the entry, outside the sticky box, so both headings stick the same.
              entry.kind === 'heading' ? 'sticky top-0 z-20 bg-surface-base' : entry.kind === 'project' ? 'sticky top-[30px] z-10 bg-surface-base phone:top-[42px]' : '',
              entry.id === 'projects-heading' ? 'mt-4' : '',
              // A project's row scrolls into view below its project header, which sticks under the 30px heading.
              entry.kind === 'conversation' && entry.projectId !== null ? 'scroll-mt-7 phone:scroll-mt-11' : '',
              drag.source.value?.id === entry.id ? 'opacity-40' : '',
              drag.target.value?.id === entry.id ? (drag.target.value.after ? 'drop-after' : 'drop-before') : '',
            ]"
            :data-sidebar-id="entry.id"
            :data-sidebar-kind="entry.kind"
            @pointerdown="entry.kind !== 'heading' && renamingId !== entry.id && drag.start($event, entry)"
          >
            <SidebarGroupLabel
              v-if="entry.kind === 'heading'"
              :label="entry.text"
              :folded="entry.folded"
              :waiting="entry.waiting"
              @toggle="toggleGroup(entry.group)"
            >
              <template #action>
                <Tooltip
                  v-if="entry.group === 'projects'"
                  content="Add project"
                  placement="right"
                >
                  <IconButton
                    :icon="FolderPlus"
                    :size="phone ? 'lg' : 'xs'"
                    variant="ghost"
                    aria-label="Add project"
                    @click="emit('addProject')"
                  />
                </Tooltip>
                <Tooltip
                  v-else
                  content="New conversation"
                  placement="right"
                >
                  <IconButton
                    :icon="SquarePen"
                    :size="phone ? 'lg' : 'xs'"
                    variant="ghost"
                    aria-label="New conversation"
                    @click="emit('create', null)"
                  />
                </Tooltip>
              </template>
            </SidebarGroupLabel>
            <!-- A line a folded group hides stays listed and folds by height, so the label's
                 group closes smoothly and the open conversation's row glides up under it. -->
            <Fold v-else :open="!entry.hidden">
              <SidebarProjectHeader
                v-if="entry.kind === 'project'"
                :project="projectById.get(entry.id)!"
                :namesake="namesakes.has(entry.id)"
                :collapsed="isFolded(entry.id)"
                :focused="list.keyboardNav.value && list.focusedId.value === entry.id"
                @toggle="list.onProjectClick(entry.id)"
                @create="emit('create', entry.id)"
                data-touch-menu
                @contextmenu="(event) => openProjectMenu(projectById.get(entry.id)!, event)"
              />
              <SidebarRow
                v-else
                :conversation="byId.get(entry.id)!"
                :open="entry.id === activeId"
                :selected="list.isSelected(entry.id)"
                :focused="list.keyboardNav.value && list.focusedId.value === entry.id"
                :menu-open="rowMenuOpenFor(entry.id)"
                :renaming="renamingId === entry.id"
                @click="(event) => list.onRowClick(entry.id, event)"
                data-touch-menu
                @contextmenu="(event) => openRowMenu(entry.id, event)"
                @archive="emit('archive', [entry.id])"
                @rename-start="renamingId = entry.id"
                @rename-submit="(title) => submitRename(entry.id, title)"
                @rename-cancel="cancelRename"
                @unpin="emit('pin', [entry.id], false)"
              />
            </Fold>
          </div>
        </TransitionGroup>
      </div>
    </ScrollArea>
    <Teleport to="body">
      <div
        v-if="drag.source.value"
        class="overlay-shell pointer-events-none fixed z-50 max-w-56 truncate rounded-md px-3 py-1 text-chrome text-fg"
        :style="{ left: `${drag.pointer.value.x + 12}px`, top: `${drag.pointer.value.y + 12}px` }"
        aria-hidden="true"
      >
        {{ drag.source.value.kind === 'project' ? projectById.get(drag.source.value.id)?.name : byId.get(drag.source.value.id)?.title }}
      </div>
    </Teleport>

    <!-- The account, signing in on a phone, and settings. -->
    <div
      class="flex shrink-0 items-center gap-1 border-t border-line px-2 py-2 phone:h-[52px] phone:py-0"
    >
      <SidebarAccountRow
        :account="account"
        @open-settings="emit('openSettings')"
        @sign-out="emit('signOut')"
      />
      <!-- A page that is itself on a phone has no phone to hand the account to. -->
      <Tooltip v-if="!phone" content="Sign In on Phone" placement="top">
        <IconButton
          :icon="Smartphone"
          :size="phone ? 'lg' : 'md'"
          variant="ghost"
          aria-label="Sign In on Phone"
          @click="emit('signInOnPhone')"
        />
      </Tooltip>
      <Tooltip content="Settings" placement="right">
        <IconButton
          :icon="Settings"
          :size="phone ? 'lg' : 'md'"
          variant="ghost"
          aria-label="Settings"
          @click="emit('openSettings')"
        />
      </Tooltip>
    </div>

    <ContextMenu :overlay-store="appOverlayStore" :owner="rowMenu">
      <template #menu>
        <SidebarSelectionMenu
          :targets="menuTargets"
          @open="(id) => emit('select', id)"
          @rename="(id) => (renamingId = id)"
          @copy-id="(id) => void copyConversationId(id)"
          @pin="(ids, pinned) => emit('pin', ids, pinned)"
          @archive="(ids) => emit('archive', ids)"
          @delete="(ids) => askDelete(ids)"
        />
      </template>
    </ContextMenu>
    <ConversationDeleteDialog
      :is-open="deleteOpen"
      :overlay-store="appOverlayStore"
      :titles="deleteTargets.map((target) => target.title)"
      @close="deleteOpen = false"
      @delete="confirmDelete"
    />
    <ContextMenu :overlay-store="appOverlayStore" :owner="projectMenu">
      <template #menu>
        <SidebarProjectMenu
          v-if="menuProject"
          :project="menuProject"
          :folded="isFolded(menuProject.id)"
          :count="groups.find((group) => group.project.id === menuProject?.id)?.items.length ?? 0"
          @create="emit('create', menuProject.id)"
          @toggle-fold="setFolded(menuProject.id, !isFolded(menuProject.id))"
          @select-all="selectProjectConversations(menuProject)"
          @remove="emit('removeProject', menuProject.id)"
        />
      </template>
    </ContextMenu>
  </aside>
</template>

<style scoped>
/* A line its folded group hides gives back its margin as its height folds. The list's own
   transitions below come later, so they win while an entry enters, leaves or moves. */
.sidebar-entry {
  transition: margin-top var(--fold-ms) ease;
}
.sidebar-items-move,
.sidebar-items-enter-active,
.sidebar-items-leave-active {
  transition: transform 180ms ease, opacity 180ms ease, height 180ms ease, margin 180ms ease;
}
.sidebar-items-enter-from,
.sidebar-items-leave-to {
  opacity: 0;
}
.sidebar-items-leave-active {
  pointer-events: none;
}
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
/* One line per boundary: "after A" and "before B" paint the same two pixels, centred on
   the 1px gap between the rows, so the line never jumps as the pointer crosses the middle. */
.drop-before::before {
  top: -2px;
}
.drop-after::after {
  bottom: -1px;
}
@media (prefers-reduced-motion: reduce) {
  .sidebar-entry,
  .sidebar-items-move,
  .sidebar-items-enter-active,
  .sidebar-items-leave-active {
    transition: none;
  }
}
</style>

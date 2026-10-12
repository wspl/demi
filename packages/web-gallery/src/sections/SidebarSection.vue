<script setup lang="ts">
import { computed, onBeforeUnmount, ref, type Ref } from 'vue'
import { moveBefore } from '@demicodes/utils'
import type { SidebarConversation, SidebarProject, SidebarProjectHost, SidebarReorder } from '@demicodes/web-ui/sidebar/types'
import type { SidebarGroup } from '@demicodes/web-ui/sidebar/list-model'
import { RestoreSweep } from '../fixtures/restore-sweep'
import type { ListLoad } from '@demicodes/web-ui/agent/session-status'
import Button from '@demicodes/web-ui/ui/Button.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySettingsFull from '../components/GallerySettingsFull.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import { SIDEBAR_WIDTH } from '@demicodes/web-ui/sidebar/sidebar-width'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import { SETTINGS_SECTIONS } from '@demicodes/web-ui/settings/sections'
import { showArchived, updateArchived } from '@demicodes/web-ui/sidebar/archived-toast'
import { showRefused } from '@demicodes/web-ui/sidebar/refused-toast'
import { dismissToast } from '@demicodes/web-ui/infra/toast'
import { APP_SHORTCUTS } from '@demicodes/web-ui/settings/shortcuts'
import type { SettingsTab } from '@demicodes/web-ui/settings/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { providePageHost, sidebarEntries, withPluginSections } from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../generated/pages'
import { createSettingsState } from '../fixtures/settings'
import { galleryPageHost, openExaSignIn, skillsPlugin, webPlugin } from '../fixtures/plugins'
import { productWould } from '../product-would'
import { demoAccount, demoConversations, demoProjects, emailOnlyAccount, longEmailAccount } from '../sidebar/sidebar-data'

const projects = ref(demoProjects())
const conversations = ref(demoConversations())
const activeId = ref<string | null>('c-login')
const collapsedProjects = ref<string[]>(['p-dotfiles'])
/** The groups folded under their label, which the product keeps per browser. */
const foldedGroups = ref<SidebarGroup[]>([])
const sidebarWidth = ref<number>(SIDEBAR_WIDTH.default)
let nextId = 1

// The live specimen's settings dialog, over the fixture the Settings view uses: an entry opens it
// on its section, and its Plugins page turns a plugin off, which takes that plugin's entry away.
const settings = createSettingsState()
providePageHost(
  galleryPageHost(
    { skills: skillsPlugin(settings.skills), web: webPlugin(settings.web) },
    { openLink: openExaSignIn },
  ),
)
const pluginOn = (plugin: string) =>
  settings.plugins.some((entry) => entry.id === plugin && entry.enabled)
/** New's keys as the keyboard settings set them by default. */
const newShortcut = APP_SHORTCUTS.find((shortcut) => shortcut.id === 'new')?.keys
const searchShortcut = APP_SHORTCUTS.find((shortcut) => shortcut.id === 'search')?.keys
const settingsSections = computed(() => withPluginSections(SETTINGS_SECTIONS, PLUGIN_PAGES, pluginOn))
const sectionEntries = computed(() => sidebarEntries(PLUGIN_PAGES, pluginOn))
const settingsOpen = ref(false)
const settingsTab = ref<SettingsTab | null>(null)

/** Opens the settings dialog, on `section` when an entry names one, as the product does. */
function openSettings(section?: string): void {
  if (section) {
    settingsTab.value = section
  }
  settingsOpen.value = true
}

// The pinned specimens: every plugin on, and the Skills plugin off.
const allEntries = sidebarEntries(PLUGIN_PAGES, () => true)
const entriesWithoutSkills = sidebarEntries(PLUGIN_PAGES, (plugin) => plugin !== 'skills')
/** What the product does with a pinned specimen's entry: settings, on the section the entry names. */
function wouldOpenSettings(section?: string): void {
  productWould(section ? `Open ${section} Settings` : 'Open Settings')
}

const anatomy: [string, string][] = [
  [
    'Top',
    'The app name, then the entries: New, then one entry for each plugin whose settings section asks for one (Skills), in the order the backend registers plugins and only while the user has the plugin on, then Archived. Every entry but New opens the settings dialog on its section. The Conversations heading carries the same New as the entry.'
  ],
  [
    'Conversations',
    'Plain conversations that run in no checkout. Manual order, pinned on top. The group folds under its label, as Projects does (Folded Groups). One on a paired device ends in that device’s mark (Device Marks): the device’s icon with a dot for its state, or without a dot once the device is removed, right of the pin when the row is pinned; a row on the Cloud shows none. On hover Archive appears to the left of the mark, over the pin’s place on a pinned row, and a pinned row shows Unpin before it; a row that is not pinned offers Pin in its menu. A new conversation shows only once its draft holds a character or a file, as New conversation; New opens an empty draft with no row, so opening it and leaving leaves nothing behind. The Conversations and Projects headings stick to the top of the list as it scrolls.'
  ],
  [
    'Projects',
    'Every checkout the agent works in, in manual order; the whole group folds under its label (Folded Groups), each ending in the host it lives on as the same mark a conversation’s row ends in (Device Marks): the Cloud icon, or the device’s icon with a dot, green while the device is online, amber while its runner updates itself, and faint while it is offline. A project shows its device’s name in words only beside another project of the same name on another device: the name stands before the mark, takes its own width, at most half of the text’s, and is cut at its end beyond that; the project’s name takes the rest and is cut only when it does not fit there; each cut name shows whole on hover. On hover the device’s name gives way and New conversation appears to the left of the mark; on a phone, where that button always shows, the name does not show. A project folds; its header reads as the group: bold, in the emphasis colour, and it sticks under the Projects heading while its rows scroll, which step in under it. An empty project offers its first conversation.'
  ],
  [
    'Row',
    'A title and one quiet dot: yellow while a permission request waits for the user and has not been read, over every other mark; otherwise the tone the conversation wears on its tab, the accent breathing while it runs, green once finished and red once failed until the user reads it, and a faint ring once a finished answer, a failure or a request is read, when stopped or when nothing has run yet (Status Dots). An answer not yet read also shows in a brighter title. A cut title ends in an ellipsis and shows whole in its tooltip. Archive appears on hover, to the left of the device’s mark, with Unpin before it on a pinned row; rename is inline. A row, like every block of the sidebar, keeps 8px from both of its edges, whether the system shows scrollbars or overlays them: the list’s scrollbar floats over that margin and takes no room.'
  ],
  [
    'Selection',
    'One selection across plain rows and projects. Click selects and opens; ⌘-click toggles; Shift-click ranges. Drag rows to reorder within a group and pin partition. Project dragging temporarily folds all projects, restores their expansion on release, and smoothly centers the moved header. Right-click acts on the selection: open, rename and copy conversation ID for one row; pin, archive and delete for any count; a conversation moves into a project from its header, where it runs. Delete asks first in a dialog that names the one conversation or counts several, and its Delete button removes the rows for good. Copy ID writes the identifier to the clipboard and confirms with a toast. Project headers have their own menu.'
  ],
  [
    'Keys',
    'The list is one tab stop. ↑↓ move and select, Shift+↑↓ extend, ⌘↑↓ jump to the ends, Space toggles, Enter opens a row or folds a project, ← → fold and unfold, ⌘A selects all, Esc collapses to the open conversation, F2 renames, ⌘⇧P pins; Alt+↑↓ reorders the focused entry. The focused entry scrolls into view, clear of the headings that stick above it.'
  ],
  [
    'Bottom',
    'The account (avatar and name), whose menu holds settings and sign-out, then Settings. Without a name the email stands in. A name too long to fit beside the avatar ends in an ellipsis; the avatar keeps its size, and Settings keeps the end of the row. The menu names the account, with the email under a name.'
  ],
  ['Search', 'Not designed yet.'],
]

function reorder(request: SidebarReorder): void {
  if (request.kind === 'project') {
    const item = projects.value.find((item) => item.id === request.id)
    const before = projects.value.find((item) => item.id === request.beforeId) ?? null
    if (item)
      projects.value = moveBefore(projects.value, item, before)
  } else {
    const item = conversations.value.find((item) => item.id === request.id)
    const before = conversations.value.find((item) => item.id === request.beforeId) ?? null
    if (item)
      conversations.value = moveBefore(conversations.value, item, before)
  }
}
function select(id: string): void {
  activeId.value = id
  conversations.value = conversations.value.map((conversation) => (
    conversation.id === id ? read(conversation) : conversation
  ))
}

/** New opens an empty draft, which the sidebar lists only once it holds a character or a file: no row is selected. */
function create(projectId: string | null): void {
  activeId.value = null
  productWould(projectId ? 'Start a New Conversation in the Project' : 'Start a New Conversation')
}

/** Adds a project and opens a new conversation in it, as the product's New Project does. */
function addProject(): void {
  const id = `p-new-${nextId++}`
  projects.value = [
    ...projects.value,
    {
      id,
      name: `project-${nextId}`,
      hostKind: 'device',
      device: { kind: 'paired', name: 'alex-mbp', state: 'online' },
      path: `/Users/alex/Projects/project-${nextId}`
    }
  ]
  create(id)
}

function patch(
  id: string,
  change: (conversation: SidebarConversation) => SidebarConversation
): void {
  conversations.value = conversations.value.map(
    (conversation) => (conversation.id === id
      ? change(conversation)
      : conversation)
  )
}

function patchMany(
  ids: string[],
  change: (conversation: SidebarConversation) => SidebarConversation
): void {
  const set = new Set(ids)
  conversations.value = conversations.value.map(
    (conversation) =>
      (set.has(conversation.id) ? change(conversation) : conversation)
  )
}

/** The rows the specimens archived, which the open one's title is still found among. */
const archivedRows = ref<SidebarConversation[]>([])
/** The answers the specimens' archives wait for, cleared when the page goes. */
const answers = new Set<ReturnType<typeof setTimeout>>()

/**
 * Archives the rows of `list` as the product does: they leave at once and the
 * toast says so, and the open one stays open. The backend's answer, which the
 * specimen gives a moment later, refuses a running row, as the backend
 * refuses running work: it comes back where it was, a toast says why, and
 * the same toast counts the others, whose Undo puts only them back.
 */
function archiveRows(list: Ref<SidebarConversation[]>, ids: string[]): void {
  const before = list.value
  const rows = before.filter((conversation) => ids.includes(conversation.id))
  list.value = without(before, ids)
  archivedRows.value = [...archivedRows.value, ...rows]
  const bringBack = (back: string[]) => {
    const kept = new Set(list.value.map((conversation) => conversation.id))
    list.value = before.filter((conversation) => kept.has(conversation.id) || back.includes(conversation.id))
    archivedRows.value = without(archivedRows.value, back)
  }
  const toast = showArchived(ids.length, () => bringBack(ids))
  const refused = rows.filter((conversation) => conversation.status === 'active')
  if (!refused.length) {
    return
  }
  const answer = setTimeout(() => {
    answers.delete(answer)
    const archived = ids.filter((id) => !refused.some((conversation) => conversation.id === id))
    bringBack(refused.map((conversation) => conversation.id))
    if (archived.length) {
      updateArchived(toast, archived.length, () => bringBack(archived))
    } else {
      dismissToast(toast)
    }
    showRefused('Archive', refused.map((conversation) => ({
      title: conversation.title,
      reasons: ['A conversation with running work cannot be archived, restored or moved'],
    })))
  }, 800)
  answers.add(answer)
}

function dropMany(ids: string[]): void {
  archiveRows(conversations, ids)
}

/** Deletes the rows the dialog asked about, as the product does: they leave the list for good. */
function deleteMany(ids: string[]): void {
  const set = new Set(ids)
  conversations.value = conversations.value.filter((conversation) => !set.has(conversation.id))
  if (activeId.value && set.has(activeId.value))
    activeId.value = conversations.value[0]?.id ?? null
}

/** The rows of `list` without `ids`, as an archive or a deletion leaves a specimen's list. */
function without(list: SidebarConversation[], ids: string[]): SidebarConversation[] {
  return list.filter((conversation) => !ids.includes(conversation.id))
}

/** Removing a project keeps its conversations as plain ones. */
function removeProject(id: string): void {
  projects.value = projects.value.filter((project) => project.id !== id)
  patchMany(
    conversations.value.filter((c) => c.projectId === id).map((c) => c.id),
    (c) => ({ ...c, projectId: null })
  )
}

/** The hover specimen's rows: pinned and not, each on the Cloud and on a paired device. */
function hoverRows(): SidebarConversation[] {
  const row = (id: string, title: string, pinned: boolean, device?: SidebarConversation['device']): SidebarConversation => ({
    id, title, pinned, device, updatedAt: '2026-10-10T09:00:00Z', status: 'done', projectId: null, unread: false,
  })
  const laptop = { kind: 'paired', name: 'MacBook Pro', state: 'online' } as const
  return [
    row('h-pinned', 'Release checklist', true),
    row('h-pinned-device', 'Profile the indexer', true, laptop),
    row('h-plain', 'Fix the login test', false),
    row('h-plain-device', 'Tidy the dotfiles', false, laptop),
  ]
}
const hoverConversations = ref(hoverRows())
const hoverActive = ref<string | null>(null)

/**
 * The device-marks specimens' projects and rows: a project alone of its name, two of one name on
 * two devices (one name long), one on a device since removed and one on the Cloud, and
 * conversations on devices in each state.
 */
function markProjects(): SidebarProject[] {
  const mac = { kind: 'paired', name: 'MacBook Pro', state: 'online' } as const
  const project = (id: string, name: string, host: SidebarProjectHost): SidebarProject => ({
    id, name, path: `/Users/alex/${name}`, ...host,
  })
  return [
    project('m-chromium', 'chromium', { hostKind: 'device', device: mac }),
    project('m-demi-mac', 'demi', { hostKind: 'device', device: mac }),
    project('m-demi-mini', 'demi', { hostKind: 'device', device: { kind: 'paired', name: 'Mac mini', state: 'online' } }),
    project('m-dotfiles-mac', 'dotfiles-and-machine-setup-scripts', { hostKind: 'device', device: { kind: 'paired', name: 'Alexs-MacBook-Pro.local', state: 'updating' } }),
    project('m-dotfiles-build', 'dotfiles-and-machine-setup-scripts', { hostKind: 'device', device: { kind: 'paired', name: 'build-01', state: 'offline' } }),
    project('m-old', 'old-experiments', { hostKind: 'device', device: { kind: 'removed' } }),
    project('m-notebooks', 'notebooks', { hostKind: 'cloud' }),
  ]
}
function markRows(): SidebarConversation[] {
  const row = (id: string, title: string, projectId: string | null, pinned: boolean, device?: SidebarConversation['device']): SidebarConversation => ({
    id, title, projectId, pinned, device, updatedAt: '2026-10-10T09:00:00Z', status: 'done', unread: false,
  })
  return [
    row('m-online', 'Profile the indexer', null, false, { kind: 'paired', name: 'MacBook Pro', state: 'online' }),
    row('m-pinned-offline', 'Release checklist', null, true, { kind: 'paired', name: 'build-01', state: 'offline' }),
    row('m-removed', 'Regex for fenced code lines', null, false, { kind: 'removed' }),
    row('m-cloud', 'How does limitShift differ from shift?', null, false),
    row('m-chromium-1', 'Takeover handoff', 'm-chromium', false),
    row('m-demi-1', 'New file icons look wrong', 'm-demi-mac', false),
  ]
}
const markProjectList = ref(markProjects())
const markConversations = ref(markRows())
const markActive = ref<string | null>(null)
const markFolded = ref<string[]>([])

function resetMarks(): void {
  markProjectList.value = markProjects()
  markConversations.value = markRows()
  markActive.value = null
}

/** Pins or unpins the rows of the marks specimens; pinned rows lead, as the product's list orders them. */
function pinMarks(ids: string[], pinned: boolean): void {
  const changed = markConversations.value.map((conversation) =>
    ids.includes(conversation.id) ? { ...conversation, pinned } : conversation)
  markConversations.value = [...changed.filter((c) => c.pinned), ...changed.filter((c) => !c.pinned)]
}

/**
 * A conversation read: its answer, its failure and its request settle to the ring, as the
 * product's read acknowledgement settles them. A request read still waits for the decision.
 */
function read(conversation: SidebarConversation): SidebarConversation {
  return { ...conversation, unread: false, needsYou: false }
}

/** A row for each state a conversation's dot shows, read and unread where it matters; a title says what happened, never whether it was read. */
function statusRows(): SidebarConversation[] {
  const row = (id: string, title: string, status: SidebarConversation['status'], unread: boolean, needsYou?: boolean): SidebarConversation => ({
    id, title, status, unread, needsYou, updatedAt: '2026-10-10T09:00:00Z', projectId: null, pinned: false,
  })
  return [
    row('s-running', 'Running', 'active', false),
    row('s-done-unread', 'Answer that came while away', 'done', true),
    row('s-done-read', 'Answer read before', 'done', false),
    row('s-failed-unread', 'Failure that came while away', 'error', true),
    row('s-failed-read', 'Failure read before', 'error', false),
    row('s-waiting-unread', 'Request that came while away', 'done', false, true),
    row('s-waiting-read', 'Request read, still waiting', 'done', false, false),
    row('s-needs-you', 'Needs you while running', 'active', false, true),
    row('s-stopped', 'Stopped', 'aborted', false),
    row('s-idle', 'Nothing run yet', 'idle', false),
  ]
}
/**
 * The folded specimen's rows: no conversation in a project waits yet, so the
 * label's dot comes only from Ask for Permission in a Project.
 */
function foldRows(): SidebarConversation[] {
  return demoConversations().map((conversation) => (
    conversation.projectId === null ? conversation : { ...conversation, needsYou: false }
  ))
}
/** The project conversation Ask for Permission in a Project makes wait, which is not the open one. */
const FOLD_ASKER = 'c-thumbs'
const foldConversations = ref(foldRows())
const foldActive = ref<string | null>('c-cookie')
const foldGroups = ref<SidebarGroup[]>(['projects'])
const foldProjects = ref<string[]>(['p-dotfiles'])
const foldAsking = computed(() => foldConversations.value.some((c) => c.id === FOLD_ASKER && c.needsYou))

/** Asks for a permission in a project conversation, or answers it, as a subagent's request and the user's decision would. */
function toggleFoldAsk(): void {
  foldConversations.value = foldConversations.value.map((c) => (c.id === FOLD_ASKER ? { ...c, needsYou: !foldAsking.value } : c))
}

function resetFold(): void {
  foldConversations.value = foldRows()
  foldActive.value = 'c-cookie'
  foldGroups.value = ['projects']
  foldProjects.value = ['p-dotfiles']
}

/** Opening a row reads it: its dot settles, and its request still waits for the user's decision. */
function selectFold(id: string): void {
  foldActive.value = id
  foldConversations.value = foldConversations.value.map((c) => (c.id === id ? read(c) : c))
}

const statusConversations = ref(statusRows())
const statusActive = ref<string | null>(null)
/** The rows a later event lights again, and what lights: an answer, a failure or a request. */
const STATUS_EVENTS: Record<string, 'unread' | 'needsYou'> = {
  's-done-read': 'unread',
  's-failed-read': 'unread',
  's-waiting-read': 'needsYou',
}
const statusUnread = computed(() => statusConversations.value.some((c) => c.unread || c.needsYou))
/** A later event lights a read row again, except the open one: the user reads that as it happens. */
const statusRelightable = computed(() => statusConversations.value.some((c) => c.id in STATUS_EVENTS && c.id !== statusActive.value && !c.unread && !c.needsYou))

/** Reads every row, as reading the conversations on another device of the user's would reach this one. */
function markStatusRead(): void {
  statusConversations.value = statusConversations.value.map(read)
}

/** A later answer, failure and request arrive: each read row lights again, the open row stays read. */
function newStatusEvents(): void {
  statusConversations.value = statusConversations.value.map((c) => {
    const lights = STATUS_EVENTS[c.id]
    if (!lights || c.id === statusActive.value) {
      return c
    }
    return { ...c, [lights]: true }
  })
}

/** Opening a row reads it, as the product acknowledges what the open conversation shows. */
function selectStatus(id: string): void {
  statusActive.value = id
  statusConversations.value = statusConversations.value.map((c) => (c.id === id ? read(c) : c))
}

function archiveStatus(ids: string[]): void {
  archiveRows(statusConversations, ids)
}

function archiveFold(ids: string[]): void {
  archiveRows(foldConversations, ids)
}

function archiveMarks(ids: string[]): void {
  archiveRows(markConversations, ids)
}

function archiveHover(ids: string[]): void {
  archiveRows(hoverConversations, ids)
}

/** Pins or unpins hover rows; pinned rows lead, as the product's list orders them. */
function pinHover(ids: string[], pinned: boolean): void {
  const changed = hoverConversations.value.map((conversation) =>
    ids.includes(conversation.id) ? { ...conversation, pinned } : conversation)
  hoverConversations.value = [...changed.filter((c) => c.pinned), ...changed.filter((c) => !c.pinned)]
}

const fixedConversations = computed(() => demoConversations())
const fixedProjects = computed(() => demoProjects())
const activeTitle = computed(() => {
  const listed = conversations.value.find((conversation) => conversation.id === activeId.value)
  if (listed) {
    return listed.title
  }
  // An archived conversation stays open, read-only, as the product shows it.
  const archived = archivedRows.value.find((conversation) => conversation.id === activeId.value)
  return archived ? `${archived.title} · Archived` : 'No conversation selected'
})

const listRestore = new RestoreSweep()
const recoveredStatus = ref<ListLoad>('failed')
const recoveredConversations = ref<SidebarConversation[]>([])

function recoveredRows(): SidebarConversation[] {
  return demoConversations().filter(
    (conversation) => conversation.projectId === 'p-demi'
  ).slice(
    0,
    5
  )
}

// Open all along, as a page load opens the conversation its address names
// whether or not the list has come.
const recoveredActive = ref<string | null>(recoveredRows()[1]?.id ?? null)

function retrySidebar(): void {
  listRestore.start((phase) => {
    recoveredStatus.value = phase
    recoveredConversations.value = phase === 'ready' ? recoveredRows() : []
  })
}

function breakSidebar(): void {
  listRestore.stop()
  recoveredStatus.value = 'failed'
  recoveredConversations.value = []
}

onBeforeUnmount(() => {
  listRestore.stop()
  for (const answer of answers) {
    clearTimeout(answer)
  }
})
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection
      title="Sidebar"
      note="The conversation list, by project. Entries at the top, the account at the bottom. Everything here is live: select, fold a project, pin, rename. Skills, Archived and Settings open the settings dialog, where turning the Skills plugin off takes its entry away."
    >
      <div class="gallery-frame divide-y divide-line">
        <div
          v-for="[name, note] in anatomy"
          :key="name"
          class="grid gap-2 px-4 py-3 md:grid-cols-[180px_1fr]"
        >
          <div class="text-[13px] text-fg">{{ name }}</div>
          <div class="text-[13px] leading-5 text-fg-muted">{{ note }}</div>
        </div>
      </div>
    </GallerySection>

    <GallerySection
      title="Expanded"
      note="The product frame: the sidebar on the base surface, the raised session beside it, and the divider between them. Drag the divider, use the arrows on it, or double-click it to return to the default. Archive acts as the product does: the rows leave and the toast says so at once, and the open conversation stays open, read-only. A moment later the specimen answers as the backend would, which refuses a running conversation: its row comes back where it was, a toast says why, and the archive’s toast then counts, and its Undo brings back, only the others."
    >
      <GallerySpecimen variant="expanded · live" wide>
        <GalleryOverlayWell size="tall" class="w-full">
        <div class="gallery-frame flex h-[40rem] w-full overflow-hidden">
          <SidebarLayout v-model:width="sidebarWidth" class="w-full">
          <template #sidebar>
          <AppSidebar
            v-model:collapsed-projects="collapsedProjects"
            v-model:folded-groups="foldedGroups"
            :account="demoAccount"
            :projects="projects"
            :conversations="conversations"
            :active-id="activeId"
            :section-entries="sectionEntries"
            :new-shortcut="newShortcut"
            :search-shortcut="searchShortcut"
            @search="productWould('Open the Search Window')"
            @reorder="reorder"
            @select="select"
            @create="create"
            @add-project="addProject"
            @remove-project="removeProject"
            @rename="(id, title) => patch(id, (c) => ({ ...c, title }))"
            @pin="(ids, pinned) => patchMany(ids, (c) => ({ ...c, pinned }))"
            @archive="dropMany"
            @delete="deleteMany"
            @open-settings="openSettings"
            @sign-out="productWould('Sign Out')"
          />
          </template>
          <div
            class="flex min-w-0 flex-1 flex-col items-center justify-center gap-1 bg-surface text-[13px] text-fg-faint"
          >
            <span>{{ activeTitle }}</span>
            <span class="font-mono text-[11px]">sidebar {{ sidebarWidth }}px · {{ SIDEBAR_WIDTH.min }}–{{ SIDEBAR_WIDTH.max }}</span>
          </div>
          <template #dialogs>
            <SettingsDialog
              v-slot="{ section }"
              v-model:tab="settingsTab"
              :is-open="settingsOpen"
              :overlay-store="appOverlayStore"
              :account="demoAccount"
              :sections="settingsSections"
              @close="settingsOpen = false"
            >
              <GallerySettingsFull :tab="section ?? ''" :state="settings" />
            </SettingsDialog>
          </template>
          </SidebarLayout>
        </div>
        </GalleryOverlayWell>
      </GallerySpecimen>
    </GallerySection>

    <GallerySection
      title="Folded Groups"
      note="Conversations and Projects each fold under their label, as Finder’s sidebar sections do. Click a label, or the chevron after it, which shows while the pointer is over the label and always on a phone: it points down while the group is open and right while it is folded. A folded group shows its label alone, but keeps the open conversation under it, in Projects with its project’s row above it so its place still shows, and while a conversation the fold hides waits for the user, the label carries the yellow dot, between the label and the chevron; the open conversation shows its own dot. Here Projects starts folded with Cookie helper refactor open; Ask for Permission in a Project makes Thumbnail pipeline on the new bucket wait until Answer the Request or until it is opened and read, which settles its dot. The product keeps the folded groups per browser, across reloads. Reset gives the specimen its rows and folds again."
    >
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="projects folded · live">
          <div class="flex flex-col gap-2">
            <div class="flex gap-2">
              <Button variant="ghost" size="sm" @click="toggleFoldAsk">{{ foldAsking ? 'Answer the Request' : 'Ask for Permission in a Project' }}</Button>
              <Button variant="ghost" size="sm" @click="resetFold">Reset</Button>
            </div>
            <div class="gallery-frame flex h-[30rem] overflow-hidden">
              <AppSidebar
                v-model:collapsed-projects="foldProjects"
                v-model:folded-groups="foldGroups"
                :new-shortcut="newShortcut"
                :search-shortcut="searchShortcut"
                :account="demoAccount"
                :projects="fixedProjects"
                :conversations="foldConversations"
                :active-id="foldActive"
                @search="productWould('Open the Search Window')"
                @create="productWould('Start a New Conversation')"
                @add-project="productWould('Add a Project')"
                @remove-project="productWould('Remove the Project')"
                @open-settings="wouldOpenSettings"
                @sign-out="productWould('Sign Out')"
                @select="selectFold"
                @reorder="(request) => productWould(request.kind === 'project' ? 'Move the Project' : 'Move the Conversation')"
                @rename="(id, title) => (foldConversations = foldConversations.map((c) => (c.id === id ? { ...c, title } : c)))"
                @pin="(ids, pinned) => (foldConversations = foldConversations.map((c) => (ids.includes(c.id) ? { ...c, pinned } : c)))"
                @archive="archiveFold"
                @delete="(ids) => (foldConversations = without(foldConversations, ids))"
              />
            </div>
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection
      title="Row Hover"
      note="Hover a row: Archive appears at its end, to the left of the device’s mark, which stays. A pinned row shows Unpin before Archive, and Archive takes the pin’s place; a row that is not pinned offers Pin in its menu, as right-click shows. Archive drops the row, and the toast’s Undo brings it back; Reset gives the specimen its rows again."
    >
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="pinned and not · live">
          <div class="flex flex-col gap-2">
            <div>
              <Button variant="ghost" size="sm" @click="hoverConversations = hoverRows()">Reset</Button>
            </div>
            <div class="gallery-frame flex h-[26rem] overflow-hidden">
              <AppSidebar
                :new-shortcut="newShortcut"
                :search-shortcut="searchShortcut"
                :account="demoAccount"
                :projects="[]"
                :conversations="hoverConversations"
                :active-id="hoverActive"
                @search="productWould('Open the Search Window')"
                @create="productWould('Start a New Conversation')"
                @add-project="productWould('Add a Project')"
                @open-settings="wouldOpenSettings"
                @sign-out="productWould('Sign Out')"
                @select="(id) => (hoverActive = id)"
                @reorder="(request) => productWould(request.kind === 'project' ? 'Move the Project' : 'Move the Conversation')"
                @rename="(id, title) => (hoverConversations = hoverConversations.map((c) => (c.id === id ? { ...c, title } : c)))"
                @pin="pinHover"
                @archive="archiveHover"
                @delete="(ids) => (hoverConversations = without(hoverConversations, ids))"
              />
            </div>
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection
      title="Device Marks"
      note="A conversation’s row and a project’s row end in the same mark, in the same column down both groups: the device’s icon with a dot for its state, green online, amber while its runner updates itself, faint offline, and no dot once the device is removed. Its tip, and its accessible label, name the device and its state, MacBook Pro · Online, or say Removed device. A project’s row names its device in words only beside a project of the same name on another device (demi on MacBook Pro and Mac mini; the two dotfiles projects, whose long names are cut at their ends); chromium alone shows the mark. A project on the Cloud shows the Cloud’s icon in the same place; a conversation on the Cloud shows none. The mark stays on hover: New conversation and Archive appear to its left, and on a project the device’s name, when it shows, disappears while the pointer is over the row, giving New conversation its place; on a phone, where the button always shows, the name does not show at all. Both frames share their rows: Reset gives them back."
    >
      <div class="flex flex-col gap-2">
        <div>
          <Button variant="ghost" size="sm" @click="resetMarks">Reset</Button>
        </div>
        <div class="specimen-row specimen-row-wide items-start">
          <GallerySpecimen
            v-for="specimen in [
              { variant: '256px · live', width: SIDEBAR_WIDTH.default },
              { variant: '200px · live', width: SIDEBAR_WIDTH.min },
            ]"
            :key="specimen.variant"
            :variant="specimen.variant"
          >
            <div class="gallery-frame flex h-[46rem] overflow-hidden" :style="{ '--sidebar-width': `${specimen.width}px` }">
              <AppSidebar
                v-model:collapsed-projects="markFolded"
                :new-shortcut="newShortcut"
                :search-shortcut="searchShortcut"
                :account="demoAccount"
                :projects="markProjectList"
                :conversations="markConversations"
                :active-id="markActive"
                @search="productWould('Open the Search Window')"
                @create="create"
                @add-project="productWould('Add a Project')"
                @remove-project="(id) => (markProjectList = markProjectList.filter((project) => project.id !== id))"
                @open-settings="wouldOpenSettings"
                @sign-out="productWould('Sign Out')"
                @select="(id) => (markActive = id)"
                @reorder="(request) => productWould(request.kind === 'project' ? 'Move the Project' : 'Move the Conversation')"
                @rename="(id, title) => (markConversations = markConversations.map((c) => (c.id === id ? { ...c, title } : c)))"
                @pin="pinMarks"
                @archive="archiveMarks"
                @delete="(ids) => (markConversations = without(markConversations, ids))"
              />
            </div>
          </GallerySpecimen>
        </div>
      </div>
    </GallerySection>

    <GallerySection
      title="Status Dots"
      note="A row’s dot says what the conversation’s work is doing, in the tones its tab and an agent’s link wear: the accent breathing while it runs, green once finished and red once failed; yellow while a request waits for the user, the same size as every other dot and with nothing around it, over every other dot. Every color but the running blue says there is something new to see: once the user reads it the row settles into the faint ring, as Mail’s unread mark does, whether it is an answer, a failure or a request; a request read but not answered still waits in its conversation, and only the dot settles. An answer not yet read also shows in a brighter title. Open a row to read it, or use Mark Read to read every row as another device of the user’s would. What happens while the user has a row open is read as it happens, so New Events lights each read row again except the open one. A later event lights the dot again: a turn that fails after the user read the last one turns it red. A Stop is the user’s own decision, and a stopped row, like one that has run nothing yet, keeps the faint ring of a settled row. An agent’s tab and link have no read state and stay green once the agent finished. Reset gives the specimen its rows again."
    >
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="every state · live">
          <div class="flex flex-col gap-2">
            <div class="flex gap-1">
              <Button variant="ghost" size="sm" :disabled="!statusUnread" @click="markStatusRead">Mark Read</Button>
              <Button variant="ghost" size="sm" :disabled="!statusRelightable" @click="newStatusEvents">New Events</Button>
              <Button variant="ghost" size="sm" @click="statusConversations = statusRows(); statusActive = null">Reset</Button>
            </div>
            <div class="gallery-frame flex h-[46rem] overflow-hidden">
              <AppSidebar
                :new-shortcut="newShortcut"
                :search-shortcut="searchShortcut"
                :account="demoAccount"
                :projects="[]"
                :conversations="statusConversations"
                :active-id="statusActive"
                @search="productWould('Open the Search Window')"
                @create="productWould('Start a New Conversation')"
                @add-project="productWould('Add a Project')"
                @open-settings="wouldOpenSettings"
                @sign-out="productWould('Sign Out')"
                @select="selectStatus"
                @reorder="(request) => productWould(request.kind === 'project' ? 'Move the Project' : 'Move the Conversation')"
                @rename="(id, title) => (statusConversations = statusConversations.map((c) => (c.id === id ? { ...c, title } : c)))"
                @pin="(ids, pinned) => (statusConversations = statusConversations.map((c) => (ids.includes(c.id) ? { ...c, pinned } : c)))"
                @archive="archiveStatus"
                @delete="(ids) => (statusConversations = without(statusConversations, ids))"
              />
            </div>
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection
      title="Plugin Entries"
      note="A plugin’s entry shows while the user has the plugin on: Skills on, then off. Each entry, like Archived and Settings, says what the product would open."
    >
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen
          v-for="specimen in [
            { variant: 'skills on', entries: allEntries },
            { variant: 'skills off', entries: entriesWithoutSkills },
          ]"
          :key="specimen.variant"
          :variant="specimen.variant"
        >
          <div class="gallery-frame flex h-[14rem] overflow-hidden">
            <AppSidebar
              :account="demoAccount"
              :projects="[]"
              :conversations="[]"
              :active-id="null"
              :section-entries="specimen.entries"
              :new-shortcut="newShortcut"
              :search-shortcut="searchShortcut"
              @search="productWould('Open the Search Window')"
              @create="productWould('Start a New Conversation')"
              @add-project="productWould('Add a Project')"
              @open-settings="wouldOpenSettings"
              @sign-out="productWould('Sign Out')"
            />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection
      title="States"
      note="Loading the list is a spinner, not a first run. Retry on failed sweeps the spinner, then the rows — not a first-run empty. The conversation open all along, as a page load opens the one its address names, is lit as soon as its row is listed. Break returns to failed. Empty is only after the list is ready."
    >
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="loading">
          <div class="gallery-frame flex h-[28rem] overflow-hidden">
            <AppSidebar
              :new-shortcut="newShortcut"
              :search-shortcut="searchShortcut"
              @search="productWould('Open the Search Window')"
              :account="demoAccount"
              :projects="fixedProjects.slice(0, 1)"
              :conversations="[]"
              :active-id="null"
              list-status="loading"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="failed · live">
          <!-- The stage lays children out in a row; the control and the frame stack in their own column. -->
          <div class="flex flex-col gap-2">
          <div>
            <Button
              variant="ghost"
              size="sm"
              :disabled="recoveredStatus === 'failed'"
              @click="breakSidebar"
            >Break</Button>
          </div>
          <div class="gallery-frame flex h-[28rem] overflow-hidden">
            <AppSidebar
              :new-shortcut="newShortcut"
              :search-shortcut="searchShortcut"
              @search="productWould('Open the Search Window')"
              :account="demoAccount"
              :projects="fixedProjects.slice(0, 1)"
              :conversations="recoveredConversations"
              :active-id="recoveredActive"
              :list-status="recoveredStatus"
              @retry-list="retrySidebar"
              @select="(id) => (recoveredActive = id)"
              @archive="(ids) => (recoveredConversations = without(recoveredConversations, ids))"
              @delete="(ids) => (recoveredConversations = without(recoveredConversations, ids))"
            />
          </div>
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="first run">
          <div class="gallery-frame flex h-[28rem] overflow-hidden">
            <AppSidebar
              :new-shortcut="newShortcut"
              :search-shortcut="searchShortcut"
              @search="productWould('Open the Search Window')"
              :account="demoAccount"
              :projects="fixedProjects.slice(0, 1)"
              :conversations="[]"
              :active-id="null"
              @create="create"
            />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection
      title="Account"
      note="The foot of the sidebar at its narrowest and at its default width. The email that stands in for a missing name fits at 256px but not at 200px, where it ends in an ellipsis beside the avatar, with Settings still at the row’s end; a longer email, as a new master account has, ends in one at 256px too. The account menu and Settings say what the product would open."
    >
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen
          v-for="specimen in [
            { variant: '200px · a name', width: SIDEBAR_WIDTH.min, account: demoAccount },
            { variant: '200px · only an email', width: SIDEBAR_WIDTH.min, account: emailOnlyAccount },
            { variant: '256px · only an email', width: SIDEBAR_WIDTH.default, account: emailOnlyAccount },
            { variant: '256px · a long email', width: SIDEBAR_WIDTH.default, account: longEmailAccount },
          ]"
          :key="specimen.variant"
          :variant="specimen.variant"
        >
          <div class="gallery-frame flex h-[20rem] overflow-hidden" :style="{ '--sidebar-width': `${specimen.width}px` }">
            <AppSidebar
              :new-shortcut="newShortcut"
              :search-shortcut="searchShortcut"
              @search="productWould('Open the Search Window')"
              :account="specimen.account"
              :projects="[]"
              :conversations="[]"
              :active-id="null"
              @create="productWould('Start a New Conversation')"
              @add-project="productWould('Add a Project')"
              @open-settings="wouldOpenSettings"
              @sign-out="productWould('Sign Out')"
            />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

  </div>
</template>

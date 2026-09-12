<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { moveBefore } from '@demicodes/utils'
import type { SidebarConversation, SidebarReorder } from '@demicodes/web-ui/sidebar/types'
import { RestoreSweep } from '@demicodes/web-ui/agent/session-restore'
import type { ListLoad } from '@demicodes/web-ui/agent/session-status'
import Button from '@demicodes/web-ui/ui/Button.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import { SIDEBAR_WIDTH } from '@demicodes/web-ui/sidebar/sidebar-width'
import { demoAccount, demoConversations, demoProjects } from '../sidebar/sidebar-data'

const projects = ref(demoProjects())
const conversations = ref(demoConversations())
const opened = ref<string | null>(null)
const activeId = ref<string | null>('c-login')
const collapsedProjects = ref<string[]>(['p-dotfiles'])
const sidebarWidth = ref<number>(SIDEBAR_WIDTH.default)
const emptyList = ref<SidebarConversation[]>([])
let nextId = 1

const anatomy: [string, string][] = [
  [
    'Top',
    'The app name, then the entries: New, Skills, Archived. Skills is disabled with an In development tooltip. Archived opens its settings section. The Conversations heading carries the same New as the entry.'
  ],
  [
    'Conversations',
    'Plain conversations that run in no checkout. Manual order, pinned on top. The Conversations and Projects headings stick to the top of the list as it scrolls.'
  ],
  [
    'Projects',
    'Every checkout the agent works in, in manual order, each with the host it lives on. A project folds; its header reads as the group: bold, in the emphasis colour, and it sticks under the Projects heading while its rows scroll, which step in under it. An empty project offers its first conversation.'
  ],
  [
    'Row',
    'A title and one quiet dot: breathing while running, green for a result waiting to be read, orange when the conversation needs the user. A cut title fades at the edge and plays as a marquee on hover. Pin and archive appear on hover; rename is inline.'
  ],
  [
    'Selection',
    'One selection across plain rows and projects. Click selects and opens; ⌘-click toggles; Shift-click ranges. Drag rows to reorder within a group and pin partition. Project dragging temporarily folds all projects, restores their expansion on release, and smoothly centers the moved header. Right-click acts on the selection: open and rename for one row; pin, move to a project, archive, and delete for any count. Project headers have their own menu.'
  ],
  [
    'Keys',
    'The list is one tab stop. ↑↓ move and select, Shift+↑↓ extend, ⌘↑↓ jump to the ends, Space toggles, Enter opens a row or folds a project, ← → fold and unfold, ⌘A selects all, Esc collapses to the open conversation, F2 renames, ⌫ deletes, ⌘⇧P pins; Alt+↑↓ reorders the focused entry.'
  ],
  [
    'Bottom',
    'The account (avatar and name) with settings and sign-out behind it. The name stands alone; the menu can still show the email.'
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
    conversation.id === id ? { ...conversation, unread: false } : conversation
  ))
}

function create(projectId: string | null): void {
  const id = `c-new-${nextId++}`
  conversations.value = [
    {
      id,
      title: 'New conversation',
      updatedAt: new Date().toISOString(),
      status: 'idle',
      projectId,
      pinned: false,
      unread: false
    },
    ...conversations.value,
  ]
  activeId.value = id
}

function addProject(): void {
  const id = `p-new-${nextId++}`
  projects.value = [
    ...projects.value,
    {
      id,
      name: `project-${nextId}`,
      host: 'zan-mbp',
      hostKind: 'device',
      path: `/Users/zan/Projects/project-${nextId}`
    }
  ]
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

function dropMany(ids: string[]): void {
  const set = new Set(ids)
  conversations.value = conversations.value.filter((conversation) => !set.has(conversation.id))
  if (activeId.value && set.has(activeId.value))
    activeId.value = conversations.value[0]?.id ?? null
}

/** Removing a project keeps its conversations as plain ones. */
function removeProject(id: string): void {
  projects.value = projects.value.filter((project) => project.id !== id)
  patchMany(
    conversations.value.filter((c) => c.projectId === id).map((c) => c.id),
    (c) => ({ ...c, projectId: null })
  )
}

const fixedConversations = computed(() => demoConversations())
const fixedProjects = computed(() => demoProjects())
const activeTitle = computed(
  () =>
    conversations.value.find((conversation) => conversation.id === activeId.value)?.title ??
    'No conversation selected'
)

const listRestore = new RestoreSweep()
const recoveredStatus = ref<ListLoad>('failed')
const recoveredConversations = ref<SidebarConversation[]>([])
const recoveredActive = ref<string | null>(null)

function recoveredRows(): SidebarConversation[] {
  return demoConversations().filter(
    (conversation) => conversation.projectId === 'p-demi'
  ).slice(
    0,
    5
  )
}

function retrySidebar(): void {
  listRestore.start((phase) => {
    recoveredStatus.value = phase
    if (phase === 'loading') {
      recoveredConversations.value = []
      recoveredActive.value = null
      return
    }
    const rows = recoveredRows()
    recoveredConversations.value = rows
    recoveredActive.value = rows[0]?.id ?? null
  })
}

function breakSidebar(): void {
  listRestore.stop()
  recoveredStatus.value = 'failed'
  recoveredConversations.value = []
  recoveredActive.value = null
}

onBeforeUnmount(() => listRestore.stop())
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection
      title="Sidebar"
      note="The conversation list, by project. Entries at the top, the account at the bottom. Everything here is live: select, fold a project, pin, rename. Skills and Archived report the section they would open."
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
      note="The product frame: the sidebar on the base surface, the raised session beside it, and the divider between them. Drag the divider, use the arrows on it, or double-click it to return to the default."
    >
      <GallerySpecimen variant="expanded · live" wide>
        <div class="gallery-frame flex h-[40rem] w-full overflow-hidden">
          <SidebarLayout v-model:width="sidebarWidth" class="w-full">
          <template #sidebar>
          <AppSidebar
            v-model:collapsed-projects="collapsedProjects"
            :account="demoAccount"
            :projects="projects"
            :conversations="conversations"
            :active-id="activeId"
            @reorder="reorder"
            @select="select"
            @create="create"
            @add-project="addProject"
            @remove-project="removeProject"
            @rename="(id, title) => patch(id, (c) => ({ ...c, title }))"
            @pin="(ids, pinned) => patchMany(ids, (c) => ({ ...c, pinned }))"
            @move-to-project="(ids, projectId) => patchMany(ids, (c) => ({ ...c, projectId }))"
            @archive="dropMany"
            @remove="dropMany"
            @open-settings="(section) => (opened = section ?? 'account')"
          />
          </template>
          <div
            class="flex min-w-0 flex-1 flex-col items-center justify-center gap-1 bg-surface text-[13px] text-fg-faint"
          >
            <span>{{ activeTitle }}<span v-if="opened"> · settings → {{ opened }}</span></span>
            <span class="font-mono text-[11px]">sidebar {{ sidebarWidth }}px · {{ SIDEBAR_WIDTH.min }}–{{ SIDEBAR_WIDTH.max }}</span>
          </div>
          </SidebarLayout>
        </div>
      </GallerySpecimen>
    </GallerySection>

    <GallerySection
      title="States"
      note="Loading the list is a spinner, not a first run. Retry on failed sweeps the spinner, then the rows — not a first-run empty. Break returns to failed. Empty is only after the list is ready."
    >
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="loading">
          <div class="gallery-frame flex h-[28rem] overflow-hidden">
            <AppSidebar
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
              :account="demoAccount"
              :projects="fixedProjects.slice(0, 1)"
              :conversations="recoveredConversations"
              :active-id="recoveredActive"
              :list-status="recoveredStatus"
              @retry-list="retrySidebar"
              @select="(id) => (recoveredActive = id)"
            />
          </div>
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="first run">
          <div class="gallery-frame flex h-[28rem] overflow-hidden">
            <AppSidebar
              :account="demoAccount"
              :projects="fixedProjects.slice(0, 1)"
              :conversations="emptyList"
              :active-id="null"
              @create="(projectId) => (emptyList = [{ id: 'first', title: 'New conversation', updatedAt: new Date().toISOString(), status: 'idle', projectId, pinned: false, unread: false }])"
            />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

  </div>
</template>

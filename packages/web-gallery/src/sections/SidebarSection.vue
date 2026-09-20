<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { moveBefore } from '@demicodes/utils'
import type { SidebarConversation, SidebarReorder } from '@demicodes/web-ui/sidebar/types'
import { RestoreSweep } from '../fixtures/restore-sweep'
import type { ListLoad } from '@demicodes/web-ui/agent/session-status'
import Button from '@demicodes/web-ui/ui/Button.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import { SIDEBAR_WIDTH } from '@demicodes/web-ui/sidebar/sidebar-width'
import { demoAccount, demoConversations, demoProjects, emailOnlyAccount } from '../sidebar/sidebar-data'

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
    'A title and one quiet dot: breathing while running, blue for a result waiting to be read, orange when the conversation needs the user. A cut title fades at the edge and plays as a marquee on hover. Pin and archive appear on hover; rename is inline.'
  ],
  [
    'Selection',
    'One selection across plain rows and projects. Click selects and opens; ⌘-click toggles; Shift-click ranges. Drag rows to reorder within a group and pin partition. Project dragging temporarily folds all projects, restores their expansion on release, and smoothly centers the moved header. Right-click acts on the selection: open, rename and copy conversation ID for one row; pin, move to a project and archive for any count; a conversation is archived, never deleted. Copy ID writes the identifier to the clipboard and confirms with a toast. Project headers have their own menu.'
  ],
  [
    'Keys',
    'The list is one tab stop. ↑↓ move and select, Shift+↑↓ extend, ⌘↑↓ jump to the ends, Space toggles, Enter opens a row or folds a project, ← → fold and unfold, ⌘A selects all, Esc collapses to the open conversation, F2 renames, ⌘⇧P pins; Alt+↑↓ reorders the focused entry.'
  ],
  [
    'Bottom',
    'The account (avatar and name), whose menu holds settings and sign-out, then Settings. Without a name the email stands in. A name too long to fit beside the avatar hides whole, leaving the avatar, and Settings keeps the end of the row. The menu names the account, with the email under a name.'
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
      note="Loading the list is a spinner, not a first run. Retry on failed sweeps the spinner, then the rows — not a first-run empty. The conversation open all along, as a page load opens the one its address names, is lit as soon as its row is listed. Break returns to failed. Empty is only after the list is ready."
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

    <GallerySection
      title="Account"
      note="The foot of the sidebar at its narrowest and at its default width. The email that stands in for a missing name fits at 256px but not at 200px, where it hides and leaves the avatar, with Settings still at the row's end."
    >
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen
          v-for="specimen in [
            { variant: '200px · a name', width: SIDEBAR_WIDTH.min, account: demoAccount },
            { variant: '200px · only an email', width: SIDEBAR_WIDTH.min, account: emailOnlyAccount },
            { variant: '256px · only an email', width: SIDEBAR_WIDTH.default, account: emailOnlyAccount },
          ]"
          :key="specimen.variant"
          :variant="specimen.variant"
        >
          <div class="gallery-frame flex h-[20rem] overflow-hidden" :style="{ '--sidebar-width': `${specimen.width}px` }">
            <AppSidebar
              :account="specimen.account"
              :projects="[]"
              :conversations="[]"
              :active-id="null"
            />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

  </div>
</template>

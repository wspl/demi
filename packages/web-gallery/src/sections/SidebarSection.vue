<script setup lang="ts">
import { computed, ref } from 'vue'
import { moveBefore } from '@demicodes/utils'
import type { SidebarConversation, SidebarExtension, SidebarReorder } from '@demicodes/web-ui/sidebar/types'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import {
  demoAccount,
  demoArchived,
  demoConversations,
  demoProjects,
  demoSkills,
} from '../sidebar/sidebar-data'

const projects = ref(demoProjects())
const conversations = ref(demoConversations())
const archived = ref(demoArchived())
const skills = ref(demoSkills())
const activeId = ref<string | null>('c-login')
const collapsedProjects = ref<string[]>(['p-dotfiles'])
const emptyList = ref<SidebarConversation[]>([])
let nextId = 1

const anatomy: [string, string][] = [
  ['Top', 'The app name, then the entries: New, Skills, Archived. Skills opens a flyout that toggles skills in place; browsing goes to its own surface. Archived opens a searchable menu to the right; choosing a conversation brings it back and opens it. The Conversations heading carries the same New as the entry.'],
  ['Conversations', 'Plain conversations that run in no checkout. Manual order, pinned on top.'],
  ['Projects', 'Every checkout the agent works in, in manual order, each with the host it lives on. A project folds; its rows sit at the same inset as plain conversations. An empty project offers its first conversation.'],
  ['Row', 'A title and one quiet dot: breathing while running, green for a result waiting to be read, orange when the conversation needs the user. A cut title fades at the edge and plays as a marquee on hover. Pin and archive appear on hover; rename is inline.'],
  ['Selection', 'One selection across plain rows and projects. Click selects and opens; ⌘-click toggles; Shift-click ranges. Drag rows to reorder within a group and pin partition. Project dragging temporarily folds all projects, restores their expansion on release, and smoothly centers the moved header. Right-click acts on the selection: open and rename for one row; pin, move to a project, archive, and delete for any count. Project headers have their own menu.'],
  ['Keys', 'The list is one tab stop. ↑↓ move and select, Shift+↑↓ extend, ⌘↑↓ jump to the ends, Space toggles, Enter opens a row or folds a project, ← → fold and unfold, ⌘A selects all, Esc collapses to the open conversation, F2 renames, ⌫ deletes, ⌘⇧P pins; Alt+↑↓ reorders the focused entry.'],
  ['Bottom', 'The account (avatar, name, plan) with settings and sign-out behind it, and Settings itself.'],
  ['Search', 'Not designed yet.'],
]

function reorder(request: SidebarReorder): void {
  if (request.kind === 'project') {
    const item = projects.value.find((item) => item.id === request.id)
    const before = projects.value.find((item) => item.id === request.beforeId) ?? null
    if (item) projects.value = moveBefore(projects.value, item, before)
  } else {
    const item = conversations.value.find((item) => item.id === request.id)
    const before = conversations.value.find((item) => item.id === request.beforeId) ?? null
    if (item) conversations.value = moveBefore(conversations.value, item, before)
  }
}
function select(id: string): void {
  activeId.value = id
  conversations.value = conversations.value.map((conversation) => (
    conversation.id === id ? { ...conversation, unread: false } : conversation
  ))
}

/** Restoring moves the conversation back to the top of the list and opens it. */
function restore(id: string): void {
  const item = archived.value.find((conversation) => conversation.id === id)
  if (!item) return
  archived.value = archived.value.filter((conversation) => conversation.id !== id)
  conversations.value = [item, ...conversations.value]
  activeId.value = id
}

function create(projectId: string | null): void {
  const id = `c-new-${nextId++}`
  conversations.value = [
    { id, title: 'New conversation', updatedAt: new Date().toISOString(), status: 'idle', projectId, pinned: false, unread: false },
    ...conversations.value,
  ]
  activeId.value = id
}

function addProject(): void {
  const id = `p-new-${nextId++}`
  projects.value = [...projects.value, { id, name: `project-${nextId}`, host: 'zan-mbp', hostKind: 'device', path: `/Users/zan/Projects/project-${nextId}` }]
}

function patch(id: string, change: (conversation: SidebarConversation) => SidebarConversation): void {
  conversations.value = conversations.value.map((conversation) => (conversation.id === id ? change(conversation) : conversation))
}

function toggle(list: SidebarExtension[], id: string, enabled: boolean): SidebarExtension[] {
  return list.map((item) => (item.id === id ? { ...item, enabled } : item))
}

function patchMany(ids: string[], change: (conversation: SidebarConversation) => SidebarConversation): void {
  const set = new Set(ids)
  conversations.value = conversations.value.map((conversation) => (set.has(conversation.id) ? change(conversation) : conversation))
}

function dropMany(ids: string[]): void {
  const set = new Set(ids)
  conversations.value = conversations.value.filter((conversation) => !set.has(conversation.id))
  if (activeId.value && set.has(activeId.value)) activeId.value = conversations.value[0]?.id ?? null
}

/** Removing a project keeps its conversations as plain ones. */
function removeProject(id: string): void {
  projects.value = projects.value.filter((project) => project.id !== id)
  patchMany(conversations.value.filter((c) => c.projectId === id).map((c) => c.id), (c) => ({ ...c, projectId: null }))
}

const fixedConversations = computed(() => demoConversations())
const fixedProjects = computed(() => demoProjects())
const activeTitle = computed(() => conversations.value.find((conversation) => conversation.id === activeId.value)?.title ?? 'No conversation selected')
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection title="Sidebar" note="The conversation list, by project. Entries at the top, the account at the bottom. Everything here is live: select, fold a project, pin, rename, toggle a skill, restore from Archived.">
      <div class="gallery-frame divide-y divide-line">
        <div v-for="[name, note] in anatomy" :key="name" class="grid gap-2 px-4 py-3 md:grid-cols-[180px_1fr]">
          <div class="text-[13px] text-fg">{{ name }}</div>
          <div class="text-[13px] leading-5 text-fg-muted">{{ note }}</div>
        </div>
      </div>
    </GallerySection>

    <GallerySection title="Expanded" note="Default width. The sidebar sits on the base surface; the session next to it is raised.">
      <GallerySpecimen variant="expanded · live" wide>
        <div class="gallery-frame flex h-[40rem] w-full overflow-hidden">
          <AppSidebar
            v-model:collapsed-projects="collapsedProjects"
            :account="demoAccount"
            :projects="projects"
            :conversations="conversations"
            :active-id="activeId"
            :skills="skills"
            :archived="archived"
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
            @toggle-skill="(id, enabled) => (skills = toggle(skills, id, enabled))"
            @restore="restore"
          />
          <div class="flex min-w-0 flex-1 items-center justify-center bg-surface text-[13px] text-fg-faint">
            {{ activeTitle }}
          </div>
        </div>
      </GallerySpecimen>
    </GallerySection>

    <GallerySection title="States" note="A first run with no conversations.">
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="first run">
          <div class="gallery-frame flex h-[28rem] overflow-hidden">
            <AppSidebar
              :account="demoAccount"
              :projects="fixedProjects.slice(0, 1)"
              :conversations="emptyList"
              :active-id="null"
              :skills="skills"
              :archived="[]"
              @create="(projectId) => (emptyList = [{ id: 'first', title: 'New conversation', updatedAt: new Date().toISOString(), status: 'idle', projectId, pinned: false, unread: false }])"
            />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="Flyouts" note="Skills toggle in place under their entry. Archived opens to the right and searches as you type.">
      <div class="specimen-row specimen-row-wide items-start">
        <GalleryOverlayWell size="wide">
          <GallerySpecimen variant="skills · pinned">
            <div class="gallery-frame flex h-[28rem] w-[36rem] overflow-hidden">
              <AppSidebar
                :account="demoAccount"
                :projects="fixedProjects"
                :conversations="fixedConversations"
                active-id="c-login"
                :skills="skills"
                :archived="archived"
                pinned-flyout="skills"
                @toggle-skill="(id, enabled) => (skills = toggle(skills, id, enabled))"
              />
            </div>
          </GallerySpecimen>
        </GalleryOverlayWell>
        <GalleryOverlayWell size="wide">
          <GallerySpecimen variant="archived · pinned">
            <div class="gallery-frame flex h-[28rem] w-[36rem] overflow-hidden">
              <AppSidebar
                :account="demoAccount"
                :projects="fixedProjects"
                :conversations="fixedConversations"
                active-id="c-login"
                :skills="skills"
                :archived="archived"
                pinned-flyout="archived"
                @restore="restore"
              />
            </div>
          </GallerySpecimen>
        </GalleryOverlayWell>
      </div>
    </GallerySection>
  </div>
</template>

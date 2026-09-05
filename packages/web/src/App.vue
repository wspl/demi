<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'
import { Archive, PanelLeft } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import SidebarNavItem from '@demicodes/web-ui/sidebar/SidebarNavItem.vue'
import { showToast } from '@demicodes/web-ui/infra/toast'
import type { SidebarReorder } from '@demicodes/web-ui/sidebar/types'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import ToastHost from '@demicodes/web-ui/ui/ToastHost.vue'
import SettingsDialog from './settings/SettingsDialog.vue'
import TargetDialog from './targets/TargetDialog.vue'
import { useConversations } from './conversation/store'
import { useResources } from './prototype/resources'
const conversations = useConversations()
const resources = useResources()
const router = useRouter()
const route = useRoute()
const collapsed = ref(false)
const folded = ref<string[]>([])
const showArchived = ref(false)
const sidebarProjects = computed(() => resources.projects.map((project) => ({
  ...project,
  hostOnline: project.hostKind === 'cloud' || !!resources.devices.find((device) => device.id === project.deviceId)?.online,
})))
const activeId = computed(() => (typeof route.params.id === 'string' ? route.params.id : null))
const account = computed(() => ({
  name: resources.username || 'Zan',
  email: '',
  plan: 'Personal workspace',
}))
function reorder(request: SidebarReorder) {
  if (request.kind === 'project') resources.reorderProject(request.id, request.beforeId)
  else conversations.reorder(request.id, request.beforeId)
}
function open(id: string) {
  resources.sidebarOpen = false
  void router.push(`/chat/${id}`)
}
function create(projectId: string | null) {
  showArchived.value = false
  open(conversations.create(projectId))
}
function addProject() {
  resources.targetMode = 'create'
  resources.targetOpen = true
}
function removeProject(id: string) {
  if (conversations.items.some((c) => c.projectId === id)) {
    conversations.notice =
      'Move the project’s conversations to another environment before removing it.'
    return
  }
  resources.projects = resources.projects.filter((p) => p.id !== id)
}
function restore(id: string) {
  conversations.archive([id], false)
  showArchived.value = false
  open(id)
}
function signOut() {
  resources.signedIn = false
  resources.settingsOpen = false
  void router.push('/login')
}
function shortcut(event: KeyboardEvent) {
  if (!resources.signedIn || !(event.metaKey || event.ctrlKey)) return
  if (event.key === ',') {
    event.preventDefault()
    resources.settingsOpen = true
  }
  if (event.key === 'Enter' || event.key.toLowerCase() !== 'n') return
  event.preventDefault()
  create(null)
}
onMounted(() => window.addEventListener('keydown', shortcut))
onUnmounted(() => window.removeEventListener('keydown', shortcut))

watch(
  () => conversations.notice,
  (message) => {
    if (!message) return
    showToast({ title: message })
    conversations.notice = ''
  },
)
</script>

<template>
  <div v-if="route.path !== '/login'" class="flex h-full bg-surface-base text-fg">
    <div
      v-if="resources.sidebarOpen"
      class="fixed inset-0 z-30 bg-black/50 md:hidden"
      @click="resources.sidebarOpen = false"
    />
    <div
      class="h-full shrink-0"
      :class="resources.sidebarOpen ? 'fixed inset-y-0 left-0 z-40 md:static' : 'hidden md:block'"
    >
      <AppSidebar
        v-model:collapsed="collapsed"
        v-model:collapsed-projects="folded"
        :account="account"
        :projects="sidebarProjects"
        :conversations="conversations.items.filter((c) => !c.archived)"
        :active-id="activeId"
        :plugins="[]"
        :skills="[]"
        hide-extensions
        hide-delete
        @reorder="reorder"
        @select="open"
        @create="create"
        @add-project="addProject"
        @remove-project="removeProject"
        @rename="conversations.rename"
        @pin="conversations.pin"
        @move-to-project="conversations.move"
        @archive="conversations.archive"
        @open-settings="resources.settingsOpen = true"
        @sign-out="signOut"
      >
        <template #navigation>
          <div class="mt-1 px-2.5">
            <SidebarNavItem
              :icon="Archive"
              label="Archived"
              :collapsed="collapsed"
              :pressed="showArchived"
              @click="showArchived = !showArchived"
            />
          </div>
        </template>
      </AppSidebar>
    </div>
    <main class="flex min-w-0 flex-1 flex-col overflow-hidden">
      <div class="flex select-none items-center px-2 md:hidden">
        <IconButton
          :icon="PanelLeft"
          variant="ghost"
          aria-label="Open sidebar"
          @click="resources.sidebarOpen = true"
        />
        <span class="px-2 text-chrome text-fg-muted">Demi</span>
      </div>
      <section v-if="showArchived" class="flex-1 overflow-auto bg-surface p-6">
        <h1 class="mb-2 select-none text-[18px] font-medium">Archived conversations</h1>
        <p
          v-if="!conversations.items.some((c) => c.archived)"
          class="py-8 text-chrome text-fg-faint"
        >
          No archived conversations.
        </p>
        <div
          v-for="c in conversations.items.filter((c) => c.archived)"
          :key="c.id"
          class="flex items-center justify-between border-b border-line py-3"
        >
          <span>{{ c.title }}</span>
          <Button @click="restore(c.id)">Restore</Button>
        </div>
        <Button class="mt-4" variant="ghost" @click="showArchived = false">
          Back to conversations
        </Button>
      </section>
      <template v-else>
        <RouterView />
      </template>
    </main>
    <SettingsDialog v-if="resources.settingsOpen" @sign-out="signOut" />
    <TargetDialog v-if="resources.targetOpen" :conversation-id="activeId" />
  </div>
  <RouterView v-else />
  <ToastHost />
</template>

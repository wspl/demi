<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'
import { PanelLeft } from '@lucide/vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import { showToast } from '@demicodes/web-ui/infra/toast'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { matchesShortcut } from '@demicodes/web-ui/ui/shortcut'
import type { SidebarReorder } from '@demicodes/web-ui/sidebar/types'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import ToastHost from '@demicodes/web-ui/ui/ToastHost.vue'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import { useDevicePairing } from '@demicodes/web-ui/devices/pairing'
import SettingsDialog from './settings/SettingsDialog.vue'
import TargetDialog from './targets/TargetDialog.vue'
import { useConversations } from './conversation/store'
import { useResources } from './prototype/resources'
import { claimDevice, deviceInstallation } from './prototype/pairing'
const conversations = useConversations()
const resources = useResources()
const router = useRouter()
const route = useRoute()
const folded = ref<string[]>([])
// Connect new device, from the host menu or elsewhere, pairs right here rather than in settings.
const pairing = useDevicePairing(claimDevice)
watch(() => resources.pairingOpen, (wanted) => {
  if (wanted) pairing.open()
  else pairing.close()
})
const activeId = computed(() => (typeof route.params.id === 'string' ? route.params.id : null))
watch(
  () => [activeId.value, conversations.items.find((item) => item.id === activeId.value)?.unread],
  () => {
    if (activeId.value) conversations.markRead(activeId.value)
  },
  { immediate: true },
)
/** The sidebar's Skills flyout lists every skill from every source; a switch there is the settings switch. */
const skillItems = computed(() =>
  resources.settings.skillSources.flatMap((source) => source.skills.map((skill) => ({ id: skill.id, name: skill.name, summary: skill.description, enabled: skill.enabled }))),
)
function toggleSkill(id: string, enabled: boolean) {
  const skill = resources.settings.skillSources.flatMap((source) => source.skills).find((entry) => entry.id === id)
  if (skill) skill.enabled = enabled
}
function browseSkills() {
  resources.settingsTab = 'skills'
  resources.settingsOpen = true
}
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
  open(id)
}
function signOut() {
  resources.signedIn = false
  resources.settingsOpen = false
  void router.push('/login')
}
/** The bindings from the keyboard settings, by the action each one names. */
const actions: Record<string, () => void> = {
  new: () => create(null),
  sidebar: () => {
    resources.sidebarOpen = !resources.sidebarOpen
  },
  settings: () => {
    resources.settingsOpen = true
  },
}
function shortcut(event: KeyboardEvent) {
  if (!resources.signedIn) return
  const binding = resources.settings.keys.find((entry) => matchesShortcut(event, entry.keys))
  const action = binding && actions[binding.id]
  if (!action) return
  event.preventDefault()
  action()
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
        v-model:collapsed-projects="folded"
        :account="account"
        :projects="resources.projects"
        :conversations="conversations.items.filter((c) => !c.archived)"
        :active-id="activeId"
        :skills="skillItems"
        :archived="conversations.items.filter((c) => c.archived)"
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
        @toggle-skill="toggleSkill"
        @manage-skills="browseSkills"
        @restore="restore"
        @open-settings="resources.settingsOpen = true"
        @sign-out="signOut"
      />
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
      <RouterView />
    </main>
    <!-- Both stay mounted and open by state, so closing plays the dialog's leave. -->
    <SettingsDialog @sign-out="signOut" />
    <TargetDialog :conversation-id="activeId" />
    <DevicePairingDialog
      :is-open="pairing.isOpen.value"
      :overlay-store="appOverlayStore"
      :installation="deviceInstallation"
      :phase="pairing.phase.value"
      @close="resources.pairingOpen = false"
      @next="pairing.phase.value = { kind: 'code' }"
      @back="pairing.phase.value = { kind: 'setup' }"
      @submit="pairing.submit"
    />
  </div>
  <RouterView v-else />
  <ToastHost />
</template>

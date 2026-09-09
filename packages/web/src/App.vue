<script setup lang="ts">
import { computed, watch } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import { showToast } from '@demicodes/web-ui/infra/toast'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useAppShortcuts } from '@demicodes/web-ui/composables/useAppShortcuts'
import type { SidebarReorder } from '@demicodes/web-ui/sidebar/types'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import ToastHost from '@demicodes/web-ui/ui/ToastHost.vue'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import { useDevicePairing } from '@demicodes/web-ui/devices/pairing'
import { isSettingsSectionEnabled } from '@demicodes/web-ui/settings/sections'
import SettingsDialog from './settings/SettingsDialog.vue'
import TargetDialog from './targets/TargetDialog.vue'
import { useConversations } from './conversation/store'
import { useResources } from './state/resources'
import { useProduct } from './state/product'
import SessionNoticeBar from '@demicodes/web-ui/agent/SessionNoticeBar.vue'
import { useSession } from './auth/session'
import { claimDevice, deviceInstallation } from './devices/pairing'
const conversations = useConversations()
const resources = useResources()
const product = useProduct()
const router = useRouter()
const route = useRoute()
const folded = computed({
  get: () => resources.local.foldedProjects,
  set: (value) => {
    resources.local.foldedProjects = value
  },
})
// Connect new device, from the host menu or elsewhere, pairs right here rather than in settings.
const pairing = useDevicePairing(claimDevice)
watch(
  () => resources.pairingOpen,
  (wanted) => {
    if (wanted) {
      pairing.open()
    } else {
      pairing.close()
    }
  },
)
const activeId = computed(() =>
  typeof route.params.id === 'string' ? route.params.id : null,
)
watch(
  () => [
    activeId.value,
    conversations.items.find((item) => item.id === activeId.value)?.unread,
    conversations.items.find((item) => item.id === activeId.value)?.load,
  ],
  () => {
    if (activeId.value) {
      conversations.markRead(activeId.value)
    }
  },
  { immediate: true },
)
const account = computed(() => ({
  name: resources.username,
  email: resources.email,
}))
/** The sidebar's Skills and Archived entries open their settings sections. */
function openSettings(section?: string) {
  if (section && isSettingsSectionEnabled(section)) {
    resources.settingsTab = section
  }
  resources.settingsOpen = true
}
function reorder(request: SidebarReorder) {
  if (request.kind === 'project') {
    void resources.reorderProject(request.id, request.beforeId).catch((error) => {
      conversations.notice = error instanceof Error ? error.message : String(error)
    })
  } else {
    conversations.reorder(request.id, request.beforeId)
  }
}
function open(id: string) {
  resources.sidebarOpen = false
  void router.push(`/chat/${id}`)
}
async function create(projectId: string | null) {
  const id = await conversations.create(projectId)
  if (id) {
    open(id)
  }
}
function addProject() {
  resources.targetMode = 'create'
  resources.targetOpen = true
}
async function removeProject(id: string) {
  try {
    await resources.removeProject(id)
  } catch (error) {
    conversations.notice = error instanceof Error ? error.message : String(error)
  }
}
async function signOut(): Promise<void> {
  try {
    await useSession().signOut()
    // A new document releases account-scoped stores, sockets and draft data.
    window.location.replace('/login')
  } catch (error) {
    showToast({
      title: 'Could not sign out',
      message: error instanceof Error ? error.message : String(error),
      tone: 'danger',
    })
  }
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
useAppShortcuts(
  () => resources.signedIn,
  () => resources.keys,
  actions,
)

watch(
  () => conversations.notice,
  (message) => {
    if (!message) {
      return
    }
    // The store reports failed operations without replacing server state.
    showToast({
      title: message,
      tone: 'danger',
    })
    conversations.notice = ''
  },
)
</script>

<template>
  <SidebarLayout
    v-if="route.path !== '/login'"
    v-model:open="resources.sidebarOpen"
  >
    <template #sidebar>
      <AppSidebar
        v-model:collapsed-projects="folded"
        :account="account"
        :projects="resources.projects"
        :conversations="conversations.items.filter((c) => !c.archived)"
        :active-id="activeId"
        :list-status="conversations.listStatus"
        hide-delete
        @retry-list="conversations.reloadList"
        @reorder="reorder"
        @select="open"
        @create="create"
        @add-project="addProject"
        @remove-project="removeProject"
        @rename="conversations.rename"
        @pin="conversations.pin"
        @move-to-project="conversations.move"
        @archive="conversations.archive"
        @open-settings="openSettings"
        @sign-out="signOut"
      />
    </template>
    <SessionNoticeBar
      v-if="product.error"
      :label="product.error"
    />
    <RouterView />
    <template #dialogs>
      <!-- Both stay mounted and open by state, so closing plays the dialog's leave. -->
      <SettingsDialog @sign-out="signOut" />
      <TargetDialog :conversation-id="activeId" />
      <DevicePairingDialog
        :is-open="pairing.isOpen.value"
        stack
        :overlay-store="appOverlayStore"
        :installation="deviceInstallation"
        :phase="pairing.phase.value"
        @close="resources.pairingOpen = false"
        @next="pairing.phase.value = { kind: 'code' }"
        @back="pairing.phase.value = { kind: 'setup' }"
        @submit="pairing.submit"
      />
    </template>
  </SidebarLayout>
  <div
    v-else
    class="h-full"
  >
    <RouterView />
  </div>
  <ToastHost />
</template>

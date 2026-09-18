<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RouterView, useRoute, useRouter } from 'vue-router'
import AsyncRegion from '@demicodes/web-ui/ui/AsyncRegion.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useAppShortcuts } from '@demicodes/web-ui/composables/useAppShortcuts'
import type { SidebarReorder } from '@demicodes/web-ui/sidebar/types'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import ToastHost from '@demicodes/web-ui/ui/ToastHost.vue'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import { useDevicePairing } from '@demicodes/web-ui/devices/pairing'
import SettingsDialog from './settings/SettingsDialog.vue'
import TargetDialog from './targets/TargetDialog.vue'
import WorkPane from './conversation/WorkPane.vue'
import { useConversations } from './conversation/store'
import { useWorkPanel } from './conversation/work'
import { useResources } from './state/resources'
import { useSession } from './auth/session'
import { claimDevice, deviceInstallation } from './devices/pairing'
const session = useSession()
const conversations = useConversations()
const resources = useResources()
// The widths follow the dividers frame by frame; the preference takes them when a resize settles.
const sidebarWidth = ref(resources.sidebarWidth)
watch(() => resources.sidebarWidth, (width) => { sidebarWidth.value = width })
const asideShare = ref(resources.asideShare)
watch(() => resources.asideShare, (share) => { asideShare.value = share })
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
// The panel opens per conversation; the frame shows the open conversation's.
const work = useWorkPanel()
const asideOpen = computed({
  get: () => activeId.value !== null && work.stateFor(activeId.value).open,
  set: (open: boolean) => {
    if (activeId.value !== null) {
      work.setOpen(work.stateFor(activeId.value), open)
    }
  },
})
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
function reorder(request: SidebarReorder) {
  if (request.kind === 'project') {
    void resources
      .reorderProject(request.id, request.beforeId)
      .catch((error) => {
        reportError('Could not reorder projects', error, { userVisible: true })
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
  resources.targetOpen = true
}
async function removeProject(id: string) {
  try {
    await resources.removeProject(id)
  } catch (error) {
    reportError('Could not remove the project', error, { userVisible: true })
  }
}
async function signOut(): Promise<void> {
  try {
    await useSession().signOut()
    // A new document releases account-scoped stores, sockets and draft data.
    window.location.replace('/login')
  } catch (error) {
    reportError('Could not sign out', error, { userVisible: true })
  }
}
/** The bindings from the keyboard settings, by the action each one names. */
const actions: Record<string, () => void> = {
  new: () => create(null),
  sidebar: () => {
    resources.sidebarOpen = !resources.sidebarOpen
  },
  settings: () => resources.openSettings(),
}
useAppShortcuts(
  () => resources.signedIn,
  () => resources.keys,
  actions,
)

</script>

<template>
  <div
    v-if="session.current.status === 'checking' || !route.matched.length"
    class="flex h-dvh items-center justify-center"
  >
    <AsyncRegion state="loading" label="Loading Demi…" />
  </div>
  <SidebarLayout
    v-else-if="route.path !== '/login'"
    v-model:open="resources.sidebarOpen"
    v-model:width="sidebarWidth"
    v-model:aside-share="asideShare"
    v-model:aside-open="asideOpen"
    @resize-end="resources.sidebarWidth = $event"
    @aside-resize-end="resources.asideShare = $event"
  >
    <template #sidebar>
      <AppSidebar
        v-model:collapsed-projects="folded"
        :account="account"
        :projects="resources.projects"
        :conversations="conversations.items.filter((c) => !c.archived)"
        :active-id="activeId"
        :list-status="conversations.listStatus"
        :pending-ids="conversations.pendingChanges"
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
        @open-settings="resources.openSettings"
        @sign-out="signOut"
      />
    </template>
    <RouterView />
    <template #aside>
      <!-- The panel belongs to the open conversation; the frame shows it only while one is open. -->
      <WorkPane
        v-if="activeId"
        :conversation-id="activeId"
        @close="asideOpen = false"
      />
    </template>
    <template #dialogs>
      <!-- Both stay mounted and open by state, so closing plays the dialog's leave. -->
      <SettingsDialog @sign-out="signOut" />
      <TargetDialog />
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
  <div v-else class="h-full">
    <RouterView />
  </div>
  <ToastHost />
</template>

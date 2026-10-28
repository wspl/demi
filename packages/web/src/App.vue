<script setup lang="ts">
import { computed, onScopeDispose, ref, watch } from 'vue'
import { useEventListener } from '@vueuse/core'
import { RouterView, useRoute, useRouter } from 'vue-router'
import StartingScreen from '@demicodes/web-ui/ui/StartingScreen.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import { errorCode, reportError } from '@demicodes/web-ui/infra/errors'
import { t as sidebarText } from '@demicodes/web-ui/sidebar/locales/text'
import { t as authText } from './auth/locales/text'
import { reloadFor, reloadPage } from '@demicodes/web-ui/infra/build-reload'
import { pageLog } from '@demicodes/web-ui/infra/log'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import { useAppShortcuts } from '@demicodes/web-ui/composables/useAppShortcuts'
import type { SavedScope, SidebarConversation, SidebarReorder, SidebarUpdate } from '@demicodes/web-ui/sidebar/types'
import { keptComputed } from '@demicodes/web-ui/infra/keep-same'
import { updateOffered } from '@demicodes/web-ui/settings/server-upgrade'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import { followTabAttention } from '@demicodes/web-ui/sidebar/tab-attention'
import { conversationTitle } from '@demicodes/web-ui/agent/conversation-title'
import ToastHost from '@demicodes/web-ui/ui/ToastHost.vue'
import ContextMenu from '@demicodes/web-ui/ui/ContextMenu.vue'
import SelectionFieldHost from '@demicodes/web-ui/agent/comments/SelectionFieldHost.vue'
import { APP_BAR_PX } from '@demicodes/web-ui/ui/app-bar'
import UpdateFailedScreen from '@demicodes/web-ui/ui/UpdateFailedScreen.vue'
import UpdatingScreen from '@demicodes/web-ui/ui/UpdatingScreen.vue'
import { update } from './state/update'
import MediaViewer from '@demicodes/web-ui/files/MediaViewer.vue'
import { provideBlobUrl } from '@demicodes/web-ui/agent/media-source'
import { provideKeepPicture } from '@demicodes/web-ui/agent/comments/pictures'
import { provideMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import HandoffDialog from '@demicodes/web-ui/auth/HandoffDialog.vue'
import PasskeyOffer from '@demicodes/web-ui/auth/PasskeyOffer.vue'
import SearchDialog from '@demicodes/web-ui/search/SearchDialog.vue'
import type { MessageRow, MessageSource, SearchRow } from '@demicodes/web-ui/search/search'
import { provideSendTargets } from '@demicodes/web-ui/agent/send-to-conversation'
import { searchConversation, searchConversations } from './api/search'
import { useDevicePairing } from '@demicodes/web-ui/devices/pairing'
import SettingsDialog from './settings/SettingsDialog.vue'
import { pageUnderSettings, useSettingsAddress } from './settings/address'
import TargetDialog from './targets/TargetDialog.vue'
import WorkPane from './conversation/WorkPane.vue'
import ChatPage from './conversation/ChatPage.vue'
import { useConversations } from './conversation/store'
import { useConversationNavigation } from './conversation/navigation'
import { useWorkPanel } from './conversation/work'
import { useResources } from './state/resources'
import { useSession } from './auth/session'
import { claimDevice, useInstallCommands } from './devices/pairing'
import { blobUrl, keepPicture } from './api/uploads'
import { providePageHost } from '@demicodes/web-ui/plugins/page'
import { productPageHost } from './plugins/host'
import { useProduct } from './state/product'
import { usePasskeySignals } from './auth/passkeys'
import { useSessions } from './auth/handoff'
import { usePasskeyOffer } from './auth/passkey-offer'
import PasswordChoice from './auth/PasswordChoice.vue'
import { attend, attended } from './state/attention'
provideBlobUrl(blobUrl)
provideKeepPicture(keepPicture)
/** How many recent conversations the search window lists before the user types. */
const RECENT_CONVERSATIONS = 20
const product = useProduct()
// The password manager follows the account's email and passkeys.
usePasskeySignals()
providePageHost(productPageHost())
// A page of another build than the backend serves loads itself again, once
// per build; one that still gets another says so. While the update screen
// shows, only its Open Demi loads the page.
const updateFailed = ref(false)
watch(
  () => [product.newBuild, update.value] as const,
  ([newer, shows]) => {
    if (newer === null || shows !== null) {
      return
    }
    pageLog.info('build.differs', { page: import.meta.env.DEMI_WEB_BUILD ?? null, served: newer })
    if (!reloadFor(newer, reloadPage)) {
      updateFailed.value = true
    }
  },
  { immediate: true },
)
// A conversation's pictures and videos open in its work panel's Media tab,
// and in the viewer's window while no page the user has on opens that tab.
const mediaViewer = provideMediaViewer({
  reveal: () => {
    const conversationId = activeId.value
    if (conversationId === null || !work.canOpen('media')) {
      return false
    }
    work.openIn(conversationId, { intent: 'media', payload: {} })
    return true
  },
})
const session = useSession()
const conversations = useConversations()
const resources = useResources()
const passkeyOffer = usePasskeyOffer()
/**
 * The sidebar's rows: each conversation's own fields and the device it runs on. Every field is
 * named, so a field the row type gains fails to compile here, and a row reads none of the
 * conversation's other state, which changes as it streams.
 *
 * A row that says what it said stays the same object, so a change of one conversation,
 * such as its status as it starts or ends, draws its row alone again.
 */
const sidebarConversations = keptComputed(() =>
  conversations.listed.map(
    (conversation): { [Field in keyof Required<SidebarConversation>]: SidebarConversation[Field] } => ({
      id: conversation.id,
      title: conversation.title,
      updatedAt: conversation.updatedAt,
      status: conversation.status,
      projectId: conversation.projectId,
      pinned: conversation.pinned,
      unread: conversation.unread,
      needsYou: conversation.needsYou,
      device: resources.conversationDevice(conversation.target),
      scheduled: conversation.scheduled,
      worktree: conversation.worktree,
    }),
  ),
)
// The widths follow the dividers frame by frame; the preference takes them when a resize settles.
const sidebarWidth = ref(resources.sidebarWidth)
watch(() => resources.sidebarWidth, (width) => { sidebarWidth.value = width })
const asideShare = ref(resources.asideShare)
watch(() => resources.asideShare, (share) => { asideShare.value = share })
const route = useRoute()
const router = useRouter()
const settingsAddress = useSettingsAddress()
/**
 * The address the window shows: under open settings, the page they opened
 * over, which stays as it was behind the dialog (`settings/address.ts`).
 */
const pageRoute = computed(() =>
  settingsAddress.section.value === undefined ? route : router.resolve(pageUnderSettings(router)),
)
const { open, openAt, create } = useConversationNavigation()
/** The search window. */
const searchOpen = ref(false)
/** The conversation the search window opens in, as Find opens it in one; none to search every conversation. */
const searchScope = ref<string | null>(null)
function openSearch(inConversation: string | null): void {
  searchScope.value = inConversation
  searchOpen.value = true
}
const searchIn = computed<MessageSource | null>(() => {
  const id = searchScope.value
  return id === null ? null : (query, signal) => searchConversation(id, query, signal)
})
/** Hand Off to Your Phone: its code opens the page the window shows. */
const handoffOpen = ref(false)
const sessions = useSessions()
const handoffHost = computed(() => sessions.handoffHost(() => pageRoute.value.fullPath))
/** The conversations the backend has, the most recently active first. */
const activeConversations = computed<SearchRow[]>(() =>
  conversations.items
    .filter((conversation) => conversation.persistence === 'synced')
    .toSorted((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    .map((conversation) => ({
      conversationId: conversation.id,
      title: conversation.title,
      titleRanges: [],
      archived: conversation.archived,
      lastActiveAt: conversation.updatedAt,
      match: null,
    })),
)
/** What the search window lists before the user types: the conversations most recently active. */
const recentConversations = keptComputed(() => activeConversations.value.slice(0, RECENT_CONVERSATIONS))
// Send to Conversation: the search window's sources, and the words go to the end of the chosen draft.
provideSendTargets({
  recent: () => activeConversations.value,
  search: searchConversations,
  describe: (id) => {
    const row = sidebarConversations.value.find((conversation) => conversation.id === id)
    if (!row)
      return null
    return { status: row, project: resources.projects.find((project) => project.id === row.projectId)?.name ?? null }
  },
  shown: () => product.activeConversationId,
  send: (to, words) => {
    const shown = conversations.items.find((item) => item.id === product.activeConversationId)
    const id = to ?? create(shown?.projectId ?? null)
    if (to !== null)
      open(to)
    const target = conversations.items.find((item) => item.id === id)
    if (target)
      conversations.appendToDraft(target, words)
  },
})
function openSearchResult(row: SearchRow): void {
  searchOpen.value = false
  openAt(row.conversationId, null, row.match?.blockId ?? null)
}
function openFoundMessage(row: MessageRow): void {
  searchOpen.value = false
  if (searchScope.value !== null)
    openAt(searchScope.value, null, row.blockId)
}
const folded = computed({
  get: () => resources.local.foldedProjects,
  set: (value) => {
    resources.local.foldedProjects = value
  },
})
const foldedGroups = computed({
  get: () => resources.local.foldedGroups ?? [],
  set: (value) => {
    resources.local.foldedGroups = value
  },
})
// Add Device, from the host menu or elsewhere, pairs right here rather than in settings;
// started beside a device menu, it hands that menu the device it pairs.
const pairing = useDevicePairing(claimDevice)
const installCommands = useInstallCommands()
watch(
  () => resources.pairingRequest,
  (request) => {
    if (request) {
      pairing.open(request.onPaired)
    } else {
      pairing.close()
    }
  },
)
const activeId = computed(() =>
  typeof pageRoute.value.params.id === 'string' ? pageRoute.value.params.id : null,
)
/** The conversation Find searches in: the one shown, once the backend holds it and so its messages. */
const findable = computed(() => {
  const id = activeId.value
  return conversations.items.find((item) => item.id === id)?.persistence === 'synced' ? id : null
})
// The panel opens per conversation; the frame shows the open conversation's.
const work = useWorkPanel()
// Before its panel arrives: what the panel held when the conversation came on screen decides what opens it on a phone.
watch(activeId, (conversationId) => work.shown(conversationId), { immediate: true })
// What the viewer shows is the open conversation's: another's Media tab
// shows the medium it showed, found again in its own transcript.
watch(activeId, () => mediaViewer.close())
// The open conversation's tabs are read with its panel closed too, since the frame then mounts no
// panel: a tab a kind asks to show opens it.
watch(
  [activeId, () => activeId.value !== null && work.recorded(activeId.value)],
  ([conversationId]) => {
    if (conversationId !== null) {
      work.load(conversationId)
    }
  },
  { immediate: true },
)
/** A link's Open in Work Panel: a new tab of the shown conversation's browser, while a page the user has on opens one. */
const openInWorkPanel = computed(() => {
  const conversationId = activeId.value
  if (conversationId === null || !work.canOpen('web')) {
    return undefined
  }
  return (url: string) => work.openIn(conversationId, { intent: 'web', payload: { url } })
})
const asideOpen = computed({
  get: () => activeId.value !== null && work.stateFor(activeId.value).open,
  set: (open: boolean) => {
    if (activeId.value !== null) {
      work.setOpen(activeId.value, open)
    }
  },
})
// The shown conversation is read while the user attends the page, and a
// change of it is the user's action, from the sidebar, a link, Back or a
// notification.
watch(activeId, () => attend(), { immediate: true })
watch(
  [
    activeId,
    attended,
    () => conversations.items.find((item) => item.id === activeId.value)?.unread,
    () => conversations.items.find((item) => item.id === activeId.value)?.needsYou,
    () => conversations.items.find((item) => item.id === activeId.value)?.load,
  ],
  () => {
    if (activeId.value) {
      conversations.markRead(activeId.value)
    }
  },
  { immediate: true },
)
/** The page shows the signed-in product, with its sidebar; a sign-in page and a handoff code's do not. */
const productShown = computed(() => session.signedIn && route.path !== '/handoff')
// The tab says what the sidebar's dots say, and a sign-in page only its name.
onScopeDispose(followTabAttention({
  conversations: () => (!session.signedIn ? [] : productShown.value ? sidebarConversations.value : null),
  openTitle: () => {
    const shown = productShown.value ? conversations.items.find((item) => item.id === activeId.value) : undefined
    return shown ? conversationTitle(shown.title) : null
  },
}))
/** The server's update at the sidebar's foot, which only an administrator's state carries. */
const sidebarUpdate = computed<SidebarUpdate | null>(() => {
  const upgrade = product.snapshot?.serverUpgrade
  if (!upgrade) {
    return null
  }
  if (upgrade.run.state === 'updating') {
    return { state: 'running' }
  }
  const version = updateOffered(upgrade)
  return version === null ? null : { state: 'available', version }
})
const account = computed(() => ({
  id: resources.userId,
  name: resources.username,
  email: resources.email,
}))
function reorder(request: SidebarReorder) {
  if (request.kind === 'project') {
    void resources
      .reorderProject(request.id, request.beforeId)
      .catch((error) => {
        // A move naming a project another page deleted meanwhile has
        // nothing to say; the channel brings the order there is.
        if (errorCode(error) !== 'project_not_found') {
          reportError(sidebarText('refused.reorderProjects'), error, { site: 'refused.reorderProjects', userVisible: true })
        }
      })
  } else {
    conversations.reorder(request.id, request.beforeId)
  }
}
function addProject() {
  resources.targetOpen = true
}
async function removeProject(id: string) {
  try {
    await resources.removeProject(id)
  } catch (error) {
    // A project another page deleted meanwhile is deleted: that is done.
    if (errorCode(error) !== 'project_not_found') {
      reportError(sidebarText('refused.deleteProject'), error, { site: 'refused.deleteProject', userVisible: true })
    }
  }
}
async function switchSavedScope(id: string, scope: SavedScope, to: string | null) {
  try {
    await resources.switchSavedScope(id, scope, to)
  } catch (error) {
    reportError(sidebarText('refused.savedScope'), error, { site: 'refused.savedScope', userVisible: true })
  }
}
async function signOut(): Promise<void> {
  try {
    await useSession().signOut()
  } catch (error) {
    reportError(authText('login.signOutFailed'), error, { site: 'login.signOutFailed', userVisible: true })
  }
}
const phone = usePhone()
/**
 * Where the user last pressed or moved the focus, which Find finds in: Files,
 * another view of the work panel, the sidebar, or the conversation shown.
 */
const findArea = ref<'files' | 'panel' | 'sidebar' | 'conversation'>('conversation')
function noteFindArea(event: Event): void {
  const target = event.target instanceof Element ? event.target : null
  if (target?.closest('[data-file-view]'))
    findArea.value = 'files'
  else if (target?.closest('[data-work-panel]'))
    findArea.value = 'panel'
  else
    findArea.value = target?.closest('[data-sidebar]') ? 'sidebar' : 'conversation'
}
useEventListener(window, 'pointerdown', noteFindArea, { capture: true })
useEventListener(window, 'focusin', noteFindArea, { capture: true })
/** The bindings from the keyboard settings, by the action each one names. */
const actions: Record<string, () => void> = {
  new: () => create(null),
  search: () => openSearch(null),
  // A phone's sidebar is the drawer; a computer's collapses.
  sidebar: () => {
    if (phone.value)
      resources.sidebarOpen = !resources.sidebarOpen
    else
      resources.sidebarCollapsed = !resources.sidebarCollapsed
  },
  settings: () => void settingsAddress.open(),
  // The shown conversation's Files, with Search in Files over it.
  searchFiles,
  // Another view of the work panel keeps the keys, for the browser to find in the page.
  find: () => {
    if (findArea.value === 'panel')
      return false
    if (findArea.value === 'files')
      searchFiles()
    else
      openSearch(findArea.value === 'conversation' ? findable.value : null)
  },
}
function searchFiles(): void {
  const conversationId = activeId.value
  if (conversationId !== null && work.canOpen('file'))
    work.openIn(conversationId, { intent: 'file', payload: { path: '', search: true } })
}
useAppShortcuts(
  () => resources.signedIn,
  () => resources.keys,
  actions,
)

</script>

<template>
  <StartingScreen
    v-if="session.current.status === 'checking' || session.current.status === 'failed' || !route.matched.length"
    class="h-dvh"
    :connecting="product.connecting"
    :failure="session.current.status === 'failed' ? session.current.error : null"
    :on-retry="() => session.restore()"
  />
  <!-- A phone signed in opens a handoff code on its own page too, which goes on to the page the code opens. -->
  <div v-else-if="productShown" class="flex h-full flex-col">
    <SidebarLayout
      class="min-h-0 flex-1"
      v-model:open="resources.sidebarOpen"
      v-model:collapsed="resources.sidebarCollapsed"
      :toggle-shortcut="resources.keys.find((binding) => binding.id === 'sidebar')?.keys"
      v-model:width="sidebarWidth"
      v-model:aside-share="asideShare"
      v-model:aside-open="asideOpen"
      @resize-end="resources.sidebarWidth = $event"
      @aside-resize-end="resources.asideShare = $event"
    >
      <template #sidebar>
        <AppSidebar
          v-model:collapsed-projects="folded"
          v-model:folded-groups="foldedGroups"
          :account="account"
          :projects="resources.projects"
          :conversations="sidebarConversations"
          :active-id="activeId"
          :list-status="product.partLoad('conversations')"
          :section-entries="resources.sectionEntries"
          :new-shortcut="resources.keys.find((binding) => binding.id === 'new')?.keys"
          :search-shortcut="resources.keys.find((binding) => binding.id === 'search')?.keys"
          :settings-shortcut="resources.keys.find((binding) => binding.id === 'settings')?.keys"
          :update="sidebarUpdate"
          @search="openSearch(null)"
          @retry-list="conversations.reloadList"
          @reorder="reorder"
          @select="open"
          @create="create"
          @add-project="addProject"
          @remove-project="removeProject"
          @switch-saved-scope="switchSavedScope"
          @rename="conversations.rename"
          @pin="conversations.pin"
          @archive="conversations.archive"
          @delete="conversations.remove"
          @open-settings="settingsAddress.open"
          @hand-off-to-phone="handoffOpen = true"
          @sign-out="signOut"
        />
      </template>
      <!-- Signed in, every address shows the chat: its own, or the one settings opened over. -->
      <ChatPage :id="activeId ?? undefined" @find="openSearch(findable)" />
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
        <HandoffDialog
          :is-open="handoffOpen"
          :overlay-store="appOverlayStore"
          :host="handoffHost"
          :email="resources.email"
          @close="handoffOpen = false"
        />
        <PasskeyOffer
          :is-open="passkeyOffer.open"
          :overlay-store="appOverlayStore"
          :add="passkeyOffer.add"
          :email="resources.email"
          @answered="passkeyOffer.answered"
          @close="passkeyOffer.close"
        />
        <SearchDialog
          :is-open="searchOpen"
          :overlay-store="appOverlayStore"
          :recent="recentConversations"
          :search="searchConversations"
          :search-in="searchIn"
          @close="searchOpen = false"
          @open="openSearchResult"
          @open-message="openFoundMessage"
        />
        <DevicePairingDialog
          v-if="installCommands"
          :is-open="pairing.isOpen.value"
          stack
          :overlay-store="appOverlayStore"
          :install-commands="installCommands"
          :phase="pairing.phase.value"
          @close="resources.pairingRequest = null"
          @next="pairing.phase.value = { kind: 'code' }"
          @back="pairing.phase.value = { kind: 'setup' }"
          @submit="pairing.submit"
        />
      </template>
    </SidebarLayout>
  </div>
  <!-- An account still to choose its password sees the wizard over a blank page, whatever it opened. -->
  <div v-else-if="session.current.status === 'choosingPassword' && route.path !== '/handoff'" class="h-full bg-surface-base">
    <PasswordChoice />
  </div>
  <div v-else class="h-full">
    <RouterView />
  </div>
  <!-- The page's one context menu, which the root handler opens for whatever declares one. -->
  <ContextMenu :open-in-work-panel="openInWorkPanel" />
  <!-- The text selection's comment field, by a selection in a message. -->
  <SelectionFieldHost />
  <ToastHost :below="APP_BAR_PX" />
  <UpdateFailedScreen v-if="updateFailed" @reload="reloadPage" />
  <Teleport to="body">
    <UpdatingScreen v-if="update" :progress="update.progress" :ended="update.ended" @open="reloadPage" />
  </Teleport>
  <MediaViewer :viewer="mediaViewer" />
</template>

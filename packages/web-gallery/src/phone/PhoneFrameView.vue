<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, provide, reactive, ref } from 'vue'
import { useRoute } from 'vue-router'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import type { SettingsTab } from '@demicodes/web-ui/settings/types'
import { SETTINGS_SECTIONS } from '@demicodes/web-ui/settings/sections'
import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { providePageHost, sidebarEntries, withPluginSections } from '@demicodes/web-ui/plugins/page'
import { provideLiveCalls } from '@demicodes/web-ui/agent/live-calls'
import { useCallTerminals } from '@demicodes/web-ui/agent/terminals'
import { APP_SHORTCUTS } from '@demicodes/web-ui/settings/shortcuts'
import ChoosePasswordWizard from '@demicodes/web-ui/auth/ChoosePasswordWizard.vue'
import PasskeyOffer from '@demicodes/web-ui/auth/PasskeyOffer.vue'
import { deviceReportKey } from '@demicodes/web-ui/auth/device-unlock'
import { DEVICE_REPORTS } from '../fixtures/device-reports'
import GalleryComposer from '../components/GalleryComposer.vue'
import GalleryHostMenu from '../components/GalleryHostMenu.vue'
import GallerySettingsFull from '../components/GallerySettingsFull.vue'
import GalleryWorkPanel from '../components/GalleryWorkPanel.vue'
import { transcriptDemoBlocks } from '../fixtures/blocks'
import { galleryBrowser } from '../fixtures/live-browser'
import { galleryPasskeyOffer } from '../fixtures/passkey-offer'
import { galleryPasswordChoice } from '../fixtures/password-choice'
import { galleryConversationFiles } from '../fixtures/message-files'
import { galleryPageHost, openExaSignIn, skillsPlugin, webPlugin } from '../fixtures/plugins'
import { createSettingsState } from '../fixtures/settings'
import { gallerySubagents } from '../fixtures/subagents'
import { galleryTerminals } from '../fixtures/terminals'
import { galleryFiles, useGalleryWork } from '../fixtures/work-panel'
import { createGalleryWorkspace } from '../fixtures/workspace'
import { PLUGIN_PAGES } from '../generated/pages'
import { productWould } from '../product-would'
import { demoAccount, demoConversations, demoProjects } from '../sidebar/sidebar-data'
import { useTurnFlow } from '../turn-flow'
import type { SidebarConversation } from '@demicodes/web-ui/sidebar/types'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'
import { useAgentContext } from '../agent-context'

/** What a child's context card in the Agents window shows and does beside its usage. */
const agentContext = useAgentContext()

/**
 * One phone of the Phone page (`GalleryPhoneFrame`): the app frame at a
 * phone's size, live, over the gallery's fixtures, opened on a scene: a
 * turn running, the title sheet, the sidebar, the model picker, the work
 * panel on a Change, a file, its file tree or a browser tab, the Agents or Running window,
 * or Settings. The safe areas are the iPhone 15's, which the frame draws its
 * status bar and home indicator over.
 */
const route = useRoute()
const scene = typeof route.query.scene === 'string' ? route.query.scene : 'conversation'
// A scene may speak of another system than the iframe's own browser (`?system=android`).
const system = typeof route.query.system === 'string' ? DEVICE_REPORTS[route.query.system] : undefined
if (system) {
  provide(deviceReportKey, system.report)
}

const root = document.documentElement
root.style.setProperty('--safe-top', '59px')
root.style.setProperty('--safe-bottom', '34px')
onBeforeUnmount(() => {
  root.style.removeProperty('--safe-top')
  root.style.removeProperty('--safe-bottom')
})

const workspace = createGalleryWorkspace()
const work = useGalleryWork('change', { files: galleryFiles(workspace), browser: galleryBrowser([]) })
// The panel opens only where the scene shows it.
work.open.value = false
const COOKIE = `${workspace.root}/src/auth/cookie.ts`

const flow = useTurnFlow({
  id: 'gallery-phone',
  title: 'Fix the failing login test',
  cwd: workspace.root,
  blocks: transcriptDemoBlocks(),
  subagents: reactive(gallerySubagents()),
  terminals: reactive(galleryTerminals()),
})
const session = flow.state
const commandOf = useCallTerminals(() => session.terminals)
provideLiveCalls((toolUseId) => commandOf(undefined, toolUseId))

/** A file a message names opens in the panel, full screen, as the product opens it. */
const files = galleryConversationFiles(workspace.source, ({ path, line }) => {
  work.openIn({ intent: 'file', payload: line === null ? { path } : { path, line } })
})

const devices: HostDeviceOption[] = [
  { id: 'mac', name: 'MacBook Pro', state: 'online', path: 'thisComputer' },
  { id: 'studio', name: 'Studio PC', state: 'updating', path: 'localNetwork' },
  { id: 'old', name: 'Old Laptop', state: 'offline' },
]
/** The title sheet's scene runs on a Host that is updating, which the title's dot says. */
const primaryId = scene === 'title' ? 'studio' : 'mac'
const primaryHost = computed<HostMenuHost>(() => {
  const device = devices.find((item) => item.id === primaryId)!
  return { id: device.id, name: device.name, kind: 'device', state: device.state }
})

/** The passkey offer after a handoff, which the scene of that name shows: the whole screen. */
const passkeyOfferOpen = ref(scene === 'passkey')
/** Choose Your Password at a first sign-in, which the scene of that name shows: the whole screen. */
const passwordChoiceOpen = ref(scene === 'password')

const sidebarOpen = ref(false)
const projects = ref(demoProjects())
const conversations = ref(demoConversations())
const activeId = ref<string | null>('c-login')
/** The session shown is the sidebar's open row: its title sheet offers that row's menu. */
const shownListed = computed(() => conversations.value.find((conversation) => conversation.id === activeId.value))
function updateShown(change: (conversation: SidebarConversation) => SidebarConversation): void {
  conversations.value = conversations.value.map((conversation) => (conversation.id === activeId.value ? change(conversation) : conversation))
}
/** Archive and Delete take the shown row from the list; the product would then open a new conversation. */
function leaveShown(would: TitleText): void {
  conversations.value = conversations.value.filter((conversation) => conversation.id !== activeId.value)
  activeId.value = null
  productWould(would)
}

// Settings reaches the fixture's skills plugin, as the Settings page does.
const settings = createSettingsState()
providePageHost(
  galleryPageHost(
    { skills: skillsPlugin(settings.skills), web: webPlugin(settings.web) },
    { openLink: openExaSignIn },
  ),
)
const settingsOpen = ref(false)
const settingsTab = ref<SettingsTab | null>(null)
const settingsSections = computed(() =>
  withPluginSections(SETTINGS_SECTIONS, PLUGIN_PAGES, (plugin) =>
    settings.plugins.some((entry) => entry.id === plugin && entry.enabled),
  ),
)

function press(selector: string): void {
  document.querySelector<HTMLElement>(selector)?.click()
}

/** Presses the control `selector` names once the page shows it, as the user would; gives up after a few seconds. */
function pressWhenShown(selector: string, tries = 120): void {
  const element = document.querySelector<HTMLElement>(selector)
  if (element)
    element.click()
  else if (tries > 0)
    requestAnimationFrame(() => pressWhenShown(selector, tries - 1))
}

/** Presses the dock's chip whose label matches, as the user would. */
function pressChip(label: RegExp): void {
  const chip = [...document.querySelectorAll<HTMLElement>('[aria-haspopup="dialog"]')].find((element) => label.test(element.textContent ?? ''))
  chip?.click()
}

/** Taps the tab whose name matches once the window shows it, as a finger would: a tab selects as the press goes down. */
async function selectTab(name: RegExp, tries = 120): Promise<void> {
  const tab = [...document.querySelectorAll<HTMLElement>('[role=tab]')].find((element) => name.test(element.textContent ?? ''))
  if (tab) {
    tab.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0, pointerType: 'touch' }))
    await nextTick()
  } else if (tries > 0) {
    await new Promise((resolve) => requestAnimationFrame(resolve))
    await selectTab(name, tries - 1)
  }
}

// Each scene opens what it shows, as a tap would.
onMounted(async () => {
  await nextTick()
  switch (scene) {
    case 'conversation':
      flow.turn([{ type: 'text', text: 'Run the login tests again and fix what still fails.' }], {
        text: 'Run the login tests again and fix what still fails.',
        attachments: [],
      })
      break
    case 'title':
      press('[aria-label$="conversation options"]')
      break
    case 'drawer':
      sidebarOpen.value = true
      break
    case 'model':
      press('[aria-label="Model"]')
      break
    case 'change':
      work.select('change')
      work.open.value = true
      break
    case 'file':
      work.openIn({ intent: 'file', payload: { path: COOKIE } })
      work.open.value = true
      break
    case 'tree':
      work.openIn({ intent: 'file', payload: { path: COOKIE } })
      work.open.value = true
      pressWhenShown('[aria-label="Show file tree"]')
      break
    case 'browser':
      // Pages enough to outgrow the strip, which scrolls to show the last, the one shown.
      for (const url of ['https://docs.example.test/shipping', 'https://shop.example.test/cart', 'https://status.example.test/'])
        await work.agentOpens(url, false)
      await work.agentOpens('https://example.test/orders', true)
      break
    case 'agents':
      pressChip(/Agents?$/)
      break
    case 'running':
      pressChip(/Running/)
      break
    case 'agents-failed':
      // The second child's tab is selected, then the child fails: a finished child's tab stays while it is selected.
      pressChip(/Agents?$/)
      await selectTab(/Check the logout path/)
      session.subagents.find((agent) => agent.id === 'ag-logout')!.phase = 'error'
      break
    case 'settings':
      settingsOpen.value = true
      break
  }
})
</script>

<template>
  <div class="h-full">
    <SidebarLayout
      v-model:open="sidebarOpen"
      v-model:aside-open="work.open.value"
      class="h-full"
    >
      <template #sidebar>
        <AppSidebar
          :new-shortcut="APP_SHORTCUTS.find((shortcut) => shortcut.id === 'new')?.keys"
          :search-shortcut="APP_SHORTCUTS.find((shortcut) => shortcut.id === 'search')?.keys"
          :account="demoAccount"
          :projects="projects"
          :conversations="conversations"
          :active-id="activeId"
          :section-entries="sidebarEntries(PLUGIN_PAGES, () => true)"
          @search="productWould('Open Search')"
          @select="(id) => { activeId = id; sidebarOpen = false }"
          @create="productWould('Start a New Conversation')"
          @archive="(ids) => (conversations = conversations.filter((conversation) => !ids.includes(conversation.id)))"
          @delete="(ids) => (conversations = conversations.filter((conversation) => !ids.includes(conversation.id)))"
          @pin="(ids, pinned) => (conversations = conversations.map((conversation) => ids.includes(conversation.id) ? { ...conversation, pinned } : conversation))"
          @rename="(id, title) => (conversations = conversations.map((conversation) => conversation.id === id ? { ...conversation, title } : conversation))"
          @add-project="productWould('Add a Project')"
          @remove-project="(id) => (projects = projects.filter((project) => project.id !== id))"
          @open-settings="settingsOpen = true"
          @hand-off-to-phone="productWould('Open Hand Off to Your Phone')"
          @sign-out="productWould('Sign Out')"
        />
      </template>
      <ChatSession
        :conversation="session"
        has-provider
        :aside-open="work.open.value"
        :select-edit="work.selectEdit"
        :files="files"
        :primary-host="primaryHost"
        :listed="shownListed"
        :pending-submissions="flow.pendingSubmissions.value"
        :removing="flow.removing.value"
        @open-aside="work.open.value = true"
        @rename="session.title = $event"
        @pin="(pinned) => updateShown((conversation) => ({ ...conversation, pinned }))"
        @archive="leaveShown('Archive the Conversation and Open a New One')"
        @delete="leaveShown('Delete the Conversation and Open a New One')"
        @retry="flow.resume()"
        @retry-submission="flow.retrySubmission"
        @abort-subagents="flow.abortSubagents"
        @abort-subagent="flow.abortSubagent"
        @abort-terminal="flow.abortTerminal"
        @recall-queued="flow.recallQueued"
        @send-queued="flow.sendQueued"
        @recall-steer="flow.recallSteer"
        @interrupt-pending-steer="flow.interruptPendingSteer"
        v-bind="agentContext.session"
      >
        <template #workspace>
          <GalleryHostMenu :devices="devices" :primary-id="primaryId" has-messages />
        </template>
        <template #composer>
          <GalleryComposer
            placeholder="Ask anything…"
            :running="session.phase === 'running' || flow.startingTurn.value"
            :compacting="session.phase === 'compacting'"
            :usage="session.contextUsage ?? undefined"
            :flow="flow"
            @send="flow.turn"
            @steer="flow.steer"
            @queue="flow.queue"
            @stop="flow.stop"
            @compact="flow.compact"
          />
        </template>
      </ChatSession>
      <template #aside>
        <GalleryWorkPanel :work="work" @close="work.open.value = false" />
      </template>
      <template #dialogs>
        <ChoosePasswordWizard
          :is-open="passwordChoiceOpen"
          :overlay-store="appOverlayStore"
          email="bo@example.com"
          :choose="galleryPasswordChoice('saves')"
          offer
          :add="galleryPasskeyOffer('adds')"
          @answered="productWould('Remember the Answer on This Browser')"
          @done="passwordChoiceOpen = false"
          @sign-out="productWould('Sign Out and Open the Sign-In Page')"
        />
        <PasskeyOffer
          :is-open="passkeyOfferOpen"
          :overlay-store="appOverlayStore"
          :add="galleryPasskeyOffer('adds')"
          email="alex@example.com"
          @answered="productWould('Remember the Answer on This Browser')"
          @close="passkeyOfferOpen = false"
        />
        <SettingsDialog
          v-slot="{ section }"
          v-model:tab="settingsTab"
          :is-open="settingsOpen"
          :overlay-store="appOverlayStore"
          :account="{ id: demoAccount.id, name: demoAccount.name }"
          :sections="settingsSections"
          @close="settingsOpen = false"
        >
          <GallerySettingsFull :tab="section ?? ''" :state="settings" />
        </SettingsDialog>
      </template>
    </SidebarLayout>
  </div>
</template>

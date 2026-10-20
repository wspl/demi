<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import { settingsSections } from '@demicodes/web-ui/settings/sections'
import type { SettingsTab } from '@demicodes/web-ui/settings/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import AutomationsPane from '@demicodes/web-ui/automations/AutomationsPane.vue'
import AutomationList from '@demicodes/web-ui/automations/AutomationList.vue'
import AutomationEditor from '@demicodes/web-ui/automations/AutomationEditor.vue'
import { t } from '@demicodes/web-ui/automations/locales/text'
import { WEEKDAYS, type Automation, type AutomationChange, type AutomationLink, type AutomationRun } from '@demicodes/web-ui/automations/types'
import type { SidebarConversation } from '@demicodes/web-ui/sidebar/types'
import { triggerWords } from '@demicodes/web-ui/automations/words'
import type { HostMenuHost } from '@demicodes/web-ui/hosts/types'
import { RUN_TITLES, TIME_ZONES, automationConversations, automationHosts, galleryAutomations, wokenConversations } from '../fixtures/automations'
import { demoModels, demoProviders } from '../fixtures/catalog'
import { createSettingsState } from '../fixtures/settings'
import { GALLERY_MARKS } from '../fixtures/device-marks'
import { productWould } from '../product-would'
import { demoAccount, demoConversations, demoProjects } from '../sidebar/sidebar-data'
import GallerySettingsFull from './GallerySettingsFull.vue'
import GalleryRunConversation from './GalleryRunConversation.vue'
import GalleryWokenConversation from './GalleryWokenConversation.vue'

/**
 * The app frame with Automations, live: the sidebar's entry with its count
 * opens the page in the main area, a row opens its automation, a run opens its
 * conversation, whose bar goes back and moves it to Conversations.
 */
type View =
  | { kind: 'list' }
  | { kind: 'automation'; id: string }
  | { kind: 'run'; automationId: string; runId: string }
  | { kind: 'conversation'; id: string }

const props = withDefaults(defineProps<{
  /** Where the frame opens: the page, an automation, or a run's conversation. */
  start?: View
  /** The frame's height; a phone's frame fills its screen. */
  fill?: boolean
}>(), { start: () => ({ kind: 'list' }) })

const automations = ref<Automation[]>(galleryAutomations())
const hosts = automationHosts()
const wakeable = automationConversations()
const view = ref<View>(props.start)
const projects = ref(demoProjects())
const conversations = ref<SidebarConversation[]>([...wokenConversations(), ...demoConversations()])
const sidebarOpen = ref(false)
const now = ref(Date.now())

const settings = createSettingsState()
const settingsOpen = ref(false)
const settingsTab = ref<SettingsTab | null>(null)
/** No browser has notifications on: here, the one browser the specimen stands for. */
const notificationsOff = computed(() => !(settings.notifications.enabled && settings.notifications.permission === 'granted'))

/** Turn On Notifications opens Settings › Notifications, where the note goes once they are on. */
function turnOnNotifications(): void {
  settingsTab.value = 'notifications'
  settingsOpen.value = true
}

/** The secret Show shows, by automation, as the backend would answer it. */
const secrets = ref<Record<string, string>>({})
const SECRETS: Record<string, string> = {
  'a-pulls': 'dhs_q7Lm2VxR9tKp4WcZ8bNf3HsJ6yGd5EaU',
  'a-refunds': 'dhs_T3kP8wX2mQ7nR5vB9cL4jF6hD1sZ0aYe',
}

const current = computed(() => {
  const shown = view.value
  return shown.kind === 'automation' || shown.kind === 'run'
    ? automations.value.find((each) => each.id === (shown.kind === 'run' ? shown.automationId : shown.id)) ?? null
    : null
})
const currentRun = computed(() => {
  const shown = view.value
  return shown.kind === 'run' ? current.value?.runs.find((each) => each.id === shown.runId) ?? null : null
})
/** The automations that wake each conversation, which its row's bolt and its chip name. */
function wakersOf(id: string): Automation[] {
  return automations.value.filter((each) => each.target.kind === 'existing' && each.target.conversationId === id)
}
function linksOf(id: string): AutomationLink[] {
  return wakersOf(id).map((each) => ({ id: each.id, name: each.name, trigger: triggerWords(each), paused: each.paused }))
}
const listed = computed(() => conversations.value.map((each) => {
  const links = linksOf(each.id)
  return links.length ? { ...each, automations: links } : each
}))
/** Where a woken conversation runs, which its header's Host button shows. */
function hostOf(id: string): HostMenuHost {
  const host = hosts.find((each) => each.id === wakeable.find((conversation) => conversation.id === id)?.hostId)
  return host?.kind === 'device'
    ? { id: host.id, name: host.name, kind: 'device', state: host.state, mark: GALLERY_MARKS.mac }
    : { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online', mark: null }
}

/** The sidebar's count: runs that notified and whose conversation was not opened since. */
const unread = computed(() => automations.value.reduce((sum, each) => sum + each.runs.filter((run) => run.unread).length, 0))
const activeId = computed(() => {
  const shown = view.value
  if (shown.kind === 'conversation') {
    return shown.id
  }
  return currentRun.value && !currentRun.value.filed ? currentRun.value.conversationId : null
})

function open(next: View): void {
  view.value = next
  sidebarOpen.value = false
}

function patch(id: string, change: Partial<Automation>): void {
  automations.value = automations.value.map((each) => (each.id === id ? { ...each, ...change } : each))
}

/** An edit, as the backend saves it: a Stripe secret is kept, never shown; turning on clears why it paused. */
function save(id: string, change: AutomationChange): void {
  const { stripeSecret, ...parts } = change
  const automation = automations.value.find((each) => each.id === id)
  const trigger = parts.trigger ?? automation?.trigger
  patch(id, {
    ...parts,
    ...(stripeSecret === undefined ? {} : { stripeSigning: stripeSecret !== null }),
    ...(parts.paused === false ? { pausedBecause: null } : {}),
    ...(parts.trigger ? { address: trigger?.kind === 'url' ? `https://demi.example.com/hooks/${id}` : null } : {}),
  })
}

function patchRun(automationId: string, runId: string, change: Partial<AutomationRun>): void {
  const automation = automations.value.find((each) => each.id === automationId)
  if (automation) {
    patch(automationId, { runs: automation.runs.map((run) => (run.id === runId ? { ...run, ...change } : run)) })
  }
}

/** New Automation: one at once, paused, daily at 09:00 every day in a new conversation on the Cloud, opened. */
function create(): void {
  const base = t('page.newName')
  const taken = new Set(automations.value.map((each) => each.name))
  let name = base
  for (let number = 2; taken.has(name); number += 1) {
    name = `${base} (${number})`
  }
  const id = `a-new-${Date.now()}`
  automations.value = [{
    id,
    name,
    trigger: { kind: 'time', schedule: { kind: 'daily', time: '09:00', days: [...WEEKDAYS] } },
    timeZone: TIME_ZONES[0],
    maxRunsPerDay: null,
    target: { kind: 'new', hostId: 'cloud', projectId: null },
    prompt: '',
    model: null,
    paused: true,
    pausedBecause: null,
    address: null,
    stripeSigning: false,
    nextAt: null,
    runs: [],
  }, ...automations.value]
  open({ kind: 'automation', id })
}

/** The runs Run Now started, each answering a moment later as the agent would. */
const timers = new Set<ReturnType<typeof setTimeout>>()
onBeforeUnmount(() => {
  for (const timer of timers) {
    clearTimeout(timer)
  }
  timers.clear()
})

function runNow(automation: Automation): void {
  const id = `r-now-${Date.now()}`
  const existing = automation.target.kind === 'existing' ? automation.target.conversationId : null
  const run: AutomationRun = {
    id,
    at: new Date().toISOString(),
    state: 'running',
    note: null,
    line: null,
    conversationId: existing ?? `run-${id}`,
    unread: false,
    filed: existing === null,
  }
  patch(automation.id, { runs: [run, ...automation.runs] })
  const timer = setTimeout(() => {
    timers.delete(timer)
    patchRun(automation.id, id, { state: 'done', line: 'Nothing new since the last run.' })
  }, 2500)
  timers.add(timer)
}

function remove(id: string): void {
  automations.value = automations.value.filter((each) => each.id !== id)
  open({ kind: 'list' })
}

function showSecret(id: string): void {
  secrets.value = { ...secrets.value, [id]: SECRETS[id] ?? 'dhs_N6bV1cX8zL3kJ5hG7fD2sA9qW4eR0tYu' }
}

function hideSecret(id: string): void {
  const { [id]: _hidden, ...others } = secrets.value
  secrets.value = others
}

/** Regenerate Secret: a new one, which shows, and the old one is refused from then on. */
function regenerate(id: string): void {
  const alphabet = 'abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'
  const secret = Array.from(crypto.getRandomValues(new Uint8Array(32)), (byte) => alphabet[byte % alphabet.length]).join('')
  SECRETS[id] = `dhs_${secret}`
  showSecret(id)
}

/** A run's conversation opens: an existing conversation's in the sidebar, a run's own here, which reads it. */
function openRun(automation: Automation, run: AutomationRun): void {
  if (automation.target.kind === 'existing' && run.conversationId === automation.target.conversationId) {
    open({ kind: 'conversation', id: run.conversationId })
    return
  }
  if (run.unread) {
    patchRun(automation.id, run.id, { unread: false })
  }
  open({ kind: 'run', automationId: automation.id, runId: run.id })
}

/** Move to Conversations: the run's conversation joins the sidebar among Conversations, and stays open. */
function move(automation: Automation, run: AutomationRun): void {
  if (!run.filed || !run.conversationId) {
    return
  }
  patchRun(automation.id, run.id, { filed: false })
  conversations.value = [{
    id: run.conversationId,
    title: runTitle(automation, run),
    updatedAt: new Date().toISOString(),
    status: 'idle',
    projectId: null,
    pinned: false,
    unread: false,
  }, ...conversations.value]
}

/** A run's conversation's title, which its agent gave it; a run Run Now just started has the automation's name until then. */
function runTitle(automation: Automation, run: AutomationRun): string {
  return RUN_TITLES[run.id] ?? automation.name
}

function openSettings(section?: string): void {
  if (section) {
    settingsTab.value = section
  }
  settingsOpen.value = true
}

function select(id: string): void {
  const moved = automations.value.flatMap((automation) => automation.runs.map((run) => ({ automation, run })))
    .find(({ run }) => !run.filed && run.conversationId === id && run.conversationId.startsWith('run-'))
  open(moved ? { kind: 'run', automationId: moved.automation.id, runId: moved.run.id } : { kind: 'conversation', id })
}
const selectedTitle = computed(() => {
  const shown = view.value
  return shown.kind === 'conversation' ? conversations.value.find((each) => each.id === shown.id)?.title ?? '' : ''
})
</script>

<template>
  <div class="flex w-full overflow-hidden" :class="fill ? 'h-full' : 'gallery-frame h-[44rem]'">
    <SidebarLayout v-model:open="sidebarOpen" class="w-full" :label="t('page.title')">
      <template #sidebar>
        <AppSidebar
          :account="demoAccount"
          :projects="projects"
          :conversations="listed"
          :active-id="activeId"
          :automations="{ count: unread, open: view.kind === 'list' || view.kind === 'automation' }"
          @select="select"
          @create="productWould('Open a New Conversation')"
          @search="productWould('Open the Search Window')"
          @add-project="productWould('Add a Project')"
          @rename="(id, title) => (conversations = conversations.map((each) => (each.id === id ? { ...each, title } : each)))"
          @pin="(ids, pinned) => (conversations = conversations.map((each) => (ids.includes(each.id) ? { ...each, pinned } : each)))"
          @archive="(ids) => (conversations = conversations.filter((each) => !ids.includes(each.id)))"
          @delete="(ids) => (conversations = conversations.filter((each) => !ids.includes(each.id)))"
          @reorder="productWould('Reorder the Sidebar')"
          @open-settings="openSettings"
          @open-automations="open({ kind: 'list' })"
          @hand-off-to-phone="productWould('Hand Off to Phone')"
          @retry-list="productWould('Load the Conversations Again')"
          @sign-out="productWould('Sign Out')"
        />
      </template>
      <GalleryRunConversation
        v-if="view.kind === 'run' && current && currentRun"
        :key="currentRun.id"
        :automation="current"
        :run="currentRun"
        :title="runTitle(current, currentRun)"
        @open-page="open({ kind: 'list' })"
        @open-automation="open({ kind: 'automation', id: current.id })"
        @move="move(current, currentRun)"
      />
      <AutomationsPane
        v-else-if="view.kind === 'list' || view.kind === 'automation'"
        :title="t('page.title')"
        :back="view.kind === 'automation' ? t('page.title') : undefined"
        :add="t('page.new')"
        @back="open({ kind: 'list' })"
        @add="create"
      >
        <AutomationEditor
          v-if="view.kind === 'automation' && current"
          :automation="current"
          :hosts="hosts"
          :conversations="wakeable"
          :time-zones="TIME_ZONES"
          :providers="demoProviders"
          :models="demoModels"
          :overlay-store="appOverlayStore"
          :secret="secrets[current.id] ?? null"
          :notifications-off="notificationsOff"
          @change="save(current.id, $event)"
          @back="open({ kind: 'list' })"
          @run-now="runNow(current)"
          @delete="remove(current.id)"
          @show-secret="showSecret(current.id)"
          @hide-secret="hideSecret(current.id)"
          @regenerate-secret="regenerate(current.id)"
          @open-run="openRun(current, $event)"
          @turn-on-notifications="turnOnNotifications"
        />
        <AutomationList
          v-else
          :automations="automations"
          :hosts="hosts"
          :conversations="wakeable"
          :now="now"
          @open="(id) => open({ kind: 'automation', id })"
          @create="create"
        />
      </AutomationsPane>
      <GalleryWokenConversation
        v-else-if="view.kind === 'conversation' && wakersOf(view.id).length"
        :id="view.id"
        :key="view.id"
        :title="selectedTitle"
        :wakers="wakersOf(view.id)"
        :links="linksOf(view.id)"
        :host="hostOf(view.id)"
        @open-automation="(id) => open({ kind: 'automation', id })"
      />
      <div
        v-else
        class="flex min-w-0 flex-1 flex-col items-center justify-center gap-1 bg-surface text-[13px] text-fg-faint md:rounded-tl-xl"
      >
        <span>{{ selectedTitle }}</span>
      </div>
      <template #dialogs>
        <SettingsDialog
          v-slot="{ section }"
          v-model:tab="settingsTab"
          :is-open="settingsOpen"
          :overlay-store="appOverlayStore"
          :account="demoAccount"
          :sections="settingsSections()"
          @close="settingsOpen = false"
        >
          <GallerySettingsFull :tab="section ?? ''" :state="settings" />
        </SettingsDialog>
      </template>
    </SidebarLayout>
  </div>
</template>

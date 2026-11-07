<script setup lang="ts">
import { reactive, ref } from 'vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import { provideLiveCalls } from '@demicodes/web-ui/agent/live-calls'
import { useCallTerminals } from '@demicodes/web-ui/agent/terminals'
import { provideMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { providePageHost, sidebarEntries } from '@demicodes/web-ui/plugins/page'
import { appShortcuts } from '@demicodes/web-ui/settings/shortcuts'
import GalleryComposer from '../components/GalleryComposer.vue'
import { demoModel, transcriptDemoBlocks } from '../fixtures/blocks'
import { galleryBrowser } from '../fixtures/live-browser'
import { galleryConversationFiles } from '../fixtures/message-files'
import { createGalleryProject } from '../fixtures/project'
import { twoSaved } from '../fixtures/saved'
import { gallerySchedules, switched } from '../fixtures/schedules'
import { gallerySessionWork } from '../fixtures/session-work'
import { gallerySubagents } from '../fixtures/subagents'
import { galleryTerminals } from '../fixtures/terminals'
import { frameMediaMarkdown } from '../fixtures/attachments'
import { projectWorkflows } from '../fixtures/workflows'
import { galleryFiles, useGalleryWork } from '../fixtures/work-panel'
import { PLUGIN_PAGES } from '../generated/pages'
import { productWould } from '../product-would'
import { demoAccount, demoConversations, demoProjects, savedScopeSwitched } from '../sidebar/sidebar-data'
import { useTurnFlow } from '../turn-flow'
import { useAgentContext } from '../agent-context'

/**
 * A conversation with its work in windows on a taskbar, live over the
 * gallery's fixtures, filling the browser's window as the product's page
 * does: Files, Changes, the browser, the simulators and Attachments open from
 * what the conversation names or from the taskbar's menu, and Agents, Running
 * and Services from its system buttons. The windows keep where they stand in
 * this browser, as the product's conversation keeps them for every device.
 */
const agentContext = useAgentContext()

const project = createGalleryProject()
const work = useGalleryWork(null, { files: galleryFiles(project), browser: galleryBrowser(), open: false })
// The page's host is the work's: an intent a message opens, such as a widget's link, opens in its windows.
providePageHost(work.host)
const agents = reactive(gallerySubagents())
const terminals = reactive(galleryTerminals())
const flow = useTurnFlow({
  id: 'gallery-windows',
  title: 'Fix the failing login test',
  cwd: project.root,
  blocks: [
    ...transcriptDemoBlocks(),
    { type: 'text', id: 'assistant-media', createdAt: new Date(Date.now() - 600).toISOString(), endedAt: new Date(Date.now() - 600).toISOString(), model: demoModel, text: frameMediaMarkdown },
  ],
  subagents: agents,
  terminals,
})
const session = flow.state
const sessionWork = gallerySessionWork(work, { keep: 'workspace' })
const commandOf = useCallTerminals(() => session.terminals)
provideLiveCalls((toolUseId) => commandOf(undefined, toolUseId))

const schedules = ref(gallerySchedules())

/** A file a message names opens in Files; a commit in Changes; an attachment in Attachments. */
const files = galleryConversationFiles(project.source, ({ path, line }) => {
  work.openIn({ intent: 'file', payload: line === null ? { path } : { path, line } })
}, ({ commit, root }) => {
  work.openIn({ intent: 'commit', payload: { commit, ...(root === undefined ? {} : { root }) } })
}, ({ path }) => {
  work.openIn({ intent: 'file', payload: { path, tree: true } })
}, (id, { name, size, url }) => {
  work.openIn({ intent: 'attachment', payload: { id, name, size, url } })
})

// A picture or a video a message shows opens in Attachments, as the product's does.
provideMediaViewer({
  reveal: () => {
    work.openIn({ intent: 'media', payload: {} })
    return true
  },
})

const sidebarOpen = ref(false)
const projects = ref(demoProjects())
const conversations = ref(demoConversations())
const activeConversation = ref<string | null>('c-login')
</script>

<template>
  <div class="h-dvh">
    <SidebarLayout v-model:open="sidebarOpen" class="h-full">
      <template #sidebar>
        <AppSidebar
          :new-shortcut="appShortcuts().find((shortcut) => shortcut.id === 'new')?.keys"
          :search-shortcut="appShortcuts().find((shortcut) => shortcut.id === 'search')?.keys"
          :settings-shortcut="appShortcuts().find((shortcut) => shortcut.id === 'settings')?.keys"
          :account="demoAccount"
          :projects="projects"
          :conversations="conversations"
          :active-id="activeConversation"
          :section-entries="sidebarEntries(PLUGIN_PAGES, () => true)"
          @search="productWould('Open Search')"
          @select="(id) => (activeConversation = id)"
          @create="productWould('Start a New Conversation')"
          @archive="(ids) => (conversations = conversations.filter((conversation) => !ids.includes(conversation.id)))"
          @delete="(ids) => (conversations = conversations.filter((conversation) => !ids.includes(conversation.id)))"
          @pin="(ids, on) => (conversations = conversations.map((conversation) => ids.includes(conversation.id) ? { ...conversation, pinned: on } : conversation))"
          @rename="(id, title) => (conversations = conversations.map((conversation) => conversation.id === id ? { ...conversation, title } : conversation))"
          @add-project="productWould('Add a Project')"
          @remove-project="productWould('Remove the Project')"
          @switch-saved-scope="(id, scope, to) => (projects = savedScopeSwitched(projects, id, scope, to))"
          @open-settings="productWould('Open Settings')"
          @hand-off="productWould('Open Hand Off to Another Device')"
          @sign-out="productWould('Sign Out')"
        />
      </template>
      <ChatSession
        class="min-w-0 flex-1"
        :conversation="session"
        has-provider
        :work="sessionWork"
        :schedules="schedules"
        :select-edit="work.selectEdit"
        :files="files"
        :pending-submissions="flow.pendingSubmissions.value"
        :removing="flow.removing.value"
        @rename="session.title = $event"
        @retry="flow.resume()"
        @retry-undelivered="(id) => flow.retrySubmission(id)"
        @edit-submission="flow.editSubmission"
        @delete-unsent="flow.deleteUnsent"
        @recall-queued="flow.recallQueued"
        @send-queued="flow.sendQueued"
        @recall-steer="flow.recallSteer"
        @interrupt-pending-steer="flow.interruptPendingSteer"
        @abort-subagents="flow.abortSubagents"
        @abort-subagent="flow.abortSubagent"
        @abort-terminal="productWould('End the Command')"
        @pause-schedule="(id, paused) => (schedules = schedules.map((schedule) => schedule.id === id ? switched(schedule, paused) : schedule))"
        @delete-schedule="(id) => (schedules = schedules.filter((schedule) => schedule.id !== id))"
        v-bind="agentContext.session"
      >
        <template #composer>
          <GalleryComposer
            placeholder="Ask Demi about the failing login test…"
            :running="session.phase === 'running' || flow.startingTurn.value"
            :compacting="session.phase === 'compacting'"
            :usage="session.contextUsage ?? undefined"
            :flow="flow"
            :saved="twoSaved"
            :workflows="projectWorkflows"
            @send="flow.turn"
            @steer="flow.steer"
            @queue="flow.queue"
            @stop="flow.stop"
            @compact="flow.compact"
          />
        </template>
      </ChatSession>
    </SidebarLayout>
  </div>
</template>

<script setup lang="ts">
import { computed, onUnmounted, ref, useTemplateRef, watch } from 'vue'
import { useRouter } from 'vue-router'
import { coded } from '@demicodes/web-ui/infra/errors'
import type { PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import { provideAutomationOpener } from '@demicodes/web-ui/agent/message-origins'
import AutomationRunBar from '@demicodes/web-ui/automations/AutomationRunBar.vue'
import { useAutomations } from '../automations/store'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
import { conversationPageKind, transcriptShown } from '@demicodes/web-ui/agent/session-status'
import ConversationComposer from './ConversationComposer.vue'
import { useContextCard } from './context-card'
import WorkspaceInfo from '../targets/WorkspaceInfo.vue'
import { executionFor, primaryHostOf } from '../targets/execution'
import { useConversations } from './store'
import { useConversationNavigation } from './navigation'
import { useResources } from '../state/resources'
import type { CredentialKinds } from '@demicodes/web-ui/agent/credential-kinds'
import { useExposedPortSettings } from '../settings/exposed-ports'
import { useProviderSettings } from '../settings/providers'
import { useSettingsAddress } from '../settings/address'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { useWorkPanel } from './work'
import { usePermissions } from './permissions'
import { conversationFiles } from './files'
import type { EditSelectionHandler } from '@demicodes/web-ui/agent/edit-selection'
import { useMessageAttachments } from './attachments'
import { lookupCodeFiles } from '../api/code-files'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import { FileBrowserError } from '@demicodes/web-ui/files/types'
import { readCommitHash } from '@demicodes/web-ui/files/history'
import type { MessageForkRequest } from '@demicodes/web-ui/agent/message-fork'
import type { Conversation } from '../state/types'
import { conversationShareService } from './sharing'

const store = useConversations()
const settingsAddress = useSettingsAddress()
const providerSettings = useProviderSettings()

/**
 * Continue with another account after a usage limit, without rotation: the
 * account becomes the entry's active one, as Activate in Settings makes it,
 * then the turn resumes on it (`providers.md` § Account rotation).
 */
async function continueWith(conversation: Parameters<typeof store.start>[0], providerId: string, credentialId: string): Promise<void> {
  const provider = providerSettings.providers.find((entry) => entry.id === providerId)
  if (provider && (await providerSettings.activateAccount(provider, credentialId))) {
    store.start(conversation)
  }
}
const exposedPorts = useExposedPortSettings()
/**
 * Expose Port on a turn that failed for a port of the user's device the
 * device had not exposed: the port is made sure to be exposed, which it may
 * be by now, then the turn resumes (`port-exposure.md` § Exposed ports).
 */
async function exposeAndResume(conversation: Parameters<typeof store.start>[0], deviceId: string, port: number): Promise<void> {
  if (await exposedPorts.expose(deviceId, port)) {
    store.start(conversation)
  }
}
const resources = useResources()
// The share follows the drag live in the page and reaches the preferences once it settles.
const dockWindowShare = ref(resources.dockWindowShare)
watch(() => resources.dockWindowShare, (share) => { dockWindowShare.value = share })
const product = useProduct()
const preferences = usePreferences()
const work = useWorkPanel()
const permissions = usePermissions()
/**
 * The open conversation, from the address the page shows: under open
 * settings, that is the address they opened over, not the settings' own.
 */
const props = defineProps<{ id?: string }>()
const router = useRouter()
const navigation = useConversationNavigation()
const conversation = computed(() =>
  store.items.find((c) => c.id === props.id),
)
const automations = useAutomations()
function openAutomation(id: string): void {
  void router.push(`/automations/${encodeURIComponent(id)}`)
}
// An automation's message names it in the transcript and leads to it.
provideAutomationOpener(openAutomation)
/** The automation and run the conversation is filed under, whose bar shows under the header. */
const filedRun = computed(() => {
  const filed = conversation.value?.filed
  return filed ? automations.runOf(filed) : null
})
/** The automations that wake the conversation, which the chip above the composer lists. */
const wakers = computed(() => (conversation.value ? automations.wakers(conversation.value.id) : []))
/** The header's place, which carries out a move the offline primary Host's card asks for. */
const place = useTemplateRef<InstanceType<typeof WorkspaceInfo>>('place')
/** Where the conversation runs now, live over the account's device states: Resume runs there. */
const primaryHost = computed(() =>
  conversation.value ? primaryHostOf(executionFor(conversation.value)) : null,
)
const pageKind = computed(() =>
  !props.id && store.listStatus === 'ready'
    ? 'none'
    : conversationPageKind(store.listStatus, !!conversation.value),
)
// A new conversation's address, as a reload of it shows, has nothing to read.
watch(
  () => props.id,
  (id) => {
    void store.activate(id ?? null, { newConversation: navigation.showsNewConversation() })
  },
  { immediate: true },
)
onUnmounted(() => void store.activate(null))
// The chat's own address with no conversation open starts a new one, an
// immediately typeable draft, as New does (`conversations.md` § Conversations and
// projects). Settings opened by a link over that address leave it as it is,
// since moving the address would close them.
watch(
  () => router.currentRoute.value.path === '/chat' && store.listStatus === 'ready',
  (start) => {
    if (start) {
      navigation.replaceWithNew()
    }
  },
  { immediate: true },
)
// A conversation deleted while the page shows it, by this page or another,
// gives way to a new one (`conversations.md` § Conversations and projects). Under
// open settings the address waits until they close, since moving it would
// close them.
watch(
  () => props.id !== undefined && store.deleted.has(props.id) && router.currentRoute.value.path.startsWith('/chat/'),
  (gone) => {
    if (gone) {
      navigation.replaceWithNew()
    }
  },
  { immediate: true },
)
// An address the page has no conversation for, once it knows them all,
// was a new conversation's when its history entry says so, as a reload of
// it finds: a new conversation opens again (`web-application.md` § What the
// reload opens). Any other shows that the conversation is not found.
watch(
  () => props.id !== undefined && !conversation.value && store.listStatus === 'ready' && router.currentRoute.value.path.startsWith('/chat/'),
  (missing) => {
    if (missing) {
      navigation.reopenNew()
    }
  },
  { immediate: true },
)
// A conversation the backend has a record of has permissions to read.
watch(
  () => conversation.value?.persistence === 'synced' ? conversation.value.id : null,
  (id) => {
    if (id) {
      permissions.follow(id)
    }
  },
  { immediate: true },
)
const project = computed(() =>
  resources.projects.find((p) => p.id === conversation.value?.projectId),
)
/** What each entry holds, which a failure's record names. */
const credentialKinds: CredentialKinds = (providerId) =>
  resources.providers.find((provider) => provider.id === providerId)?.kind ?? null
const hasProvider = computed(() =>
  resources.providerInfos.some(
    (p) => p.id === conversation.value?.model.providerId && p.isAvailable,
  ),
)

/**
 * A read of history the reader's scroll asked for: a failure shows as the
 * window's edge staying a loading row, which the next scroll there tries
 * again, so it is only logged.
 */
function readHistory(read: Promise<void>): void {
  read.catch((error: unknown) => console.warn('A page of the transcript was not read', error))
}

// A search result opens at its message, and a report's title at its
// command's call: the page reads the window around it once the
// conversation's latest page is there, at the first step of its opening
// (`web-application.md` § Transcript windows, § Opening a conversation shows
// in two steps).
watch(
  () => [store.reveal, !!conversation.value && transcriptShown(conversation.value.load)] as const,
  ([reveal, shown]) => {
    const current = conversation.value
    if (reveal && current && reveal.conversationId === current.id && shown) {
      readHistory(store.history.readAround(current, reveal.node, reveal.blockId))
    }
  },
  { immediate: true },
)

/** Goes to the call that started a command a report names, wherever it lies. */
function revealCommand(current: Conversation, commandId: string): void {
  readHistory(store.history.findCommand(current, commandId).then((at) => {
    store.reveal = { conversationId: current.id, ...at }
  }))
}

function saveScroll(id: string, state: PersistedScrollState | null): void {
  const item = store.items.find((item) => item.id === id)
  if (item) {
    item.scroll = state
  }
}

/** A tool call's file pill opens its edit through the `edit` intent, while a plugin opens it. */
const selectEdit = computed<EditSelectionHandler | undefined>(() => {
  const current = conversation.value
  if (!current || !work.canOpen('edit')) {
    return undefined
  }
  return (selection) => work.openIn(current.id, { intent: 'edit', payload: selection })
})

/** The attachments the conversation's messages name. */
const attachments = useMessageAttachments(() => props.id)

/**
 * The Host files the conversation's messages name: images from the files
 * the conversation's service keeps, at the version its file watch keeps
 * current, files opened through the `file` intent while a plugin opens it,
 * at their lines, commits opened through the `commit` intent, and which
 * paths of a reply's inline code are files, from the Host, whose home a
 * `~/` path starts from; and the attachments they name.
 */
const files = computed<ConversationFiles | undefined>(() => {
  const current = conversation.value
  if (!current) {
    return undefined
  }
  const contents = conversationFiles(current.id).workspace?.source.contents
  return {
    home: resources.deviceById(executionFor(current).deviceId)?.home ?? null,
    ...(contents ? { contents } : {}),
    open: work.canOpen('file')
      ? ({ path, line, root }) => work.openIn(current.id, { intent: 'file', payload: { path, ...(line === null ? {} : { line }), ...(root === undefined ? {} : { root }) } })
      : undefined,
    showInFiles: work.canOpen('file')
      ? ({ path, root }) => work.openIn(current.id, { intent: 'file', payload: { path, tree: true, ...(root === undefined ? {} : { root }) } })
      : undefined,
    openCommit: work.canOpen('commit')
      ? ({ commit, root }) => work.openIn(current.id, { intent: 'commit', payload: { commit, ...(root === undefined ? {} : { root }) } })
      : undefined,
    commitHash: async ({ commit, root }) => {
      const history = conversationFiles(current.id).history(root)
      if (!history) {
        throw new FileBrowserError('offline')
      }
      return readCommitHash(history, commit)
    },
    codeFiles: (paths) => lookupCodeFiles(current.id, paths, primaryHost.value?.state ?? 'offline'),
    ...(attachments.value ? { attachment: attachments.value } : {}),
  }
})

/** What a child's context card in the Agents window shows and opens beside its usage. */
const card = useContextCard(() => files.value?.open)

/** What the replies' comments do in the conversation's draft; an archived conversation takes none. */
const commenting = computed(() => (conversation.value ? store.commenting(conversation.value) : undefined))

/** What the header's menu and an answer's footer share with, once the conversation has a record and the page knows the backend's public address. */
const share = computed(() => {
  const current = conversation.value
  const publicUrl = product.snapshot?.publicUrl
  return current?.persistence === 'synced' && publicUrl ? conversationShareService(current.id, publicUrl) : undefined
})

async function fork(request: MessageForkRequest): Promise<void> {
  const sourceId = conversation.value?.id
  if (!sourceId) {
    throw coded(new Error('The source conversation is unavailable.'), 'source_conversation_unavailable')
  }
  const id = await store.fork(sourceId, request)
  if (props.id === sourceId) {
    await router.push(`/chat/${id}`)
  }
}
</script>

<template>
  <ChatSession
    v-if="pageKind === 'session' && conversation"
    v-model:dock-window-share="dockWindowShare"
    @dock-window-resize-end="resources.dockWindowShare = $event"
    :conversation="conversation"
    :has-provider="hasProvider"
    :credential-kinds="credentialKinds"
    :is-exposed="exposedPorts.isExposed"
    :backend-away="product.connection !== null"
    :resume-at-retry="preferences.resumeAtRetry"
    @update:resume-at-retry="preferences.update({ resumeAtRetry: $event })"
    :fork="fork"
    :share="share"
    :shares="conversation.shares"
    :select-edit="selectEdit"
    :file-pills="conversation.projectId !== null"
    :commenting="conversation.archived ? undefined : commenting"
    :files="files"
    :primary-host="primaryHost"
    :permission-requests="permissions.stateFor(conversation.id).requests"
    :deciding-permission="permissions.stateFor(conversation.id).deciding"
    :request-error="permissions.stateFor(conversation.id).error"
    @decide-permission="(id, decision, credential) => permissions.decide(conversation!.id, id, decision, credential)"
    :pending-submissions="
      conversation.pendingSends.map((pending) => ({
        id: pending.id,
        way: pending.way,
        text: pending.text,
        attachments: pending.fileIds.flatMap((id) =>
          conversation?.files.filter((file) => file.id === id) ?? [],
        ),
        comments: pending.comments,
        error: pending.error,
      }))
    "
    :removing="conversation.removing"
    @retry-submission="store.retrySend(conversation, $event)"
    @retry="store.start(conversation)"
    @continue-with="(providerId, credentialId) => conversation && continueWith(conversation, providerId, credentialId)"
    @open-providers="void settingsAddress.open('models')"
    @expose-port="(deviceId, port) => conversation && exposeAndResume(conversation, deviceId, port)"
    @rename="store.rename(conversation.id, $event)"
    :listed="conversation.persistence === 'synced' && !conversation.archived ? { pinned: conversation.pinned } : undefined"
    @pin="void store.pin([conversation.id], $event)"
    @archive="void store.archive([conversation.id])"
    @delete="void store.remove([conversation.id])"
    @retry-load="store.reloadSession(conversation.id)"
    @abort-subagents="store.abortSubagents(conversation)"
    @abort-subagent="store.abortSubagent(conversation, $event)"
    @abort-terminal="store.abortTerminal(conversation, $event)"
    @recall-queued="store.recall(conversation, $event)"
    @send-queued="store.sendQueued(conversation, $event)"
    @recall-steer="store.recall(conversation, $event)"
    @interrupt-pending-steer="store.interruptWithSteer(conversation, $event)"
    :edit-version="store.editVersion(conversation)"
    :message-edit="conversation.messageEdit"
    :aside-open="work.stateFor(conversation.id).open"
    :panel-tabs="work.stateFor(conversation.id).panel.tabs.length"
    @update:message-edit="conversation.messageEdit = $event"
    @regenerate="store.regenerate(conversation, $event)"
    @switch-version="(id, to) => store.switchVersion(conversation!, id, to)"
    @save-scroll="saveScroll"
    :reveal="store.reveal?.conversationId === conversation.id ? store.reveal : null"
    @revealed="store.reveal = null"
    @reveal="(at) => (store.reveal = { conversationId: conversation!.id, ...at })"
    :read-block="async (node, id) => { await store.history.readWhole(conversation!, node, id) }"
    :reveal-command="(id) => revealCommand(conversation!, id)"
    @read-before="readHistory(store.history.readBefore(conversation!, null))"
    @read-after="readHistory(store.history.readAfter(conversation!, null))"
    @show-latest="conversation.shownAt = null"
    @read-subagent="(id, at) => readHistory(at === 'latest' ? store.history.readLatest(conversation!, id) : store.history.readBefore(conversation!, id))"
    @open-aside="work.setOpen(work.stateFor(conversation.id), true)"
    :usage-limits="card.limitsOf"
    @compact-subagent="store.compact(conversation, $event)"
    @refresh-limits="card.refreshLimits"
    @open-instruction="card.openInstruction"
    :automations="wakers"
    @open-automation="openAutomation"
  >
    <template v-if="filedRun" #bar
      ><AutomationRunBar
        :name="filedRun.automation.name"
        :at="filedRun.run.at"
        :time-zone="filedRun.automation.timeZone"
        @open-page="router.push('/automations')"
        @open-automation="openAutomation(filedRun.automation.id)"
        @move="automations.moveToConversations(conversation.id)"
    /></template>
    <template #workspace
      ><WorkspaceInfo ref="place" :project="project" :conversation="conversation"
    /></template>
    <template #composer
      ><ConversationComposer
        :key="conversation.id"
        :conversation="conversation"
        :open-file="files?.open"
        @move-host="place?.choose($event)"
    /></template>
  </ChatSession>
  <section
    v-else
    class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-tl-xl bg-surface"
  >
    <SessionStatus
      :kind="pageKind === 'session' ? 'missing' : pageKind"
      @retry="store.reloadList()"
      @create="navigation.create(null)"
    />
  </section>
</template>

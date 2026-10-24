<script setup lang="ts">
import { computed, onUnmounted, ref, useTemplateRef, watch } from 'vue'
import { useRouter } from 'vue-router'
import { coded } from '@demicodes/web-ui/infra/errors'
import { viewAbandoned, viewOpened } from '@demicodes/web-ui/infra/view-timing'
import { loadRegionRenderers } from '@demicodes/web-ui/infra/region-load'
import type { PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import { provideCostWatch } from '@demicodes/web-ui/agent/context-cost'
import { provideHistoryReading } from '@demicodes/web-ui/agent/history-reading'
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
import { useSideQuestions } from './side-questions'
import { conversationFiles } from './files'
import type { EditSelectionHandler } from '@demicodes/web-ui/agent/edit-selection'
import { attachmentOrigin, useMessageAttachments } from './attachments'
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
 * then the turn resumes on it (`crates/backend-providers/src/llm/rotation.rs`).
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
 * be by now, then the turn resumes (`crates/backend-user-shard/src/shard/exposed_ports.rs`).
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
const sideQuestions = useSideQuestions()
/**
 * The open conversation, from the address the page shows: under open
 * settings, that is the address they opened over, not the settings' own.
 */
const props = defineProps<{ id?: string }>()
const emit = defineEmits<{
  /** Find in Conversation: the search window, searching the conversation shown. */
  find: []
}>()
const router = useRouter()
const navigation = useConversationNavigation()
const conversation = computed(() =>
  store.items.find((c) => c.id === props.id),
)
// The cards of the open conversation, the composer's and its subagents' windows', keep the window of its cost they show current.
provideCostWatch((window) => {
  const current = conversation.value
  return current ? store.watchRecentCost(current, window) : () => {}
})
// The transcripts of the open conversation show a loading row at an edge only
// while a page is being read for it.
provideHistoryReading((node) => {
  const current = conversation.value
  return current ? store.history.reading(current, node) : { before: false, after: false }
})
/** The schedules that wake the conversation, which the chip above the composer lists. */
const schedules = computed(() => conversation.value?.metadata.shown.value?.schedules ?? [])
/** The header's place, which carries out a move the offline primary Host's card asks for. */
const place = useTemplateRef<InstanceType<typeof WorkspaceInfo>>('place')
/** Where the conversation runs now, live over the account's device states: Resume runs there. */
const primaryHost = computed(() =>
  conversation.value ? primaryHostOf(executionFor(conversation.value)) : null,
)
/**
 * Whether the conversations are known: an address that names none yet waits
 * for the drafts the web browser stored, which may hold it, and no other
 * address waits for them.
 */
const listStatus = computed(() =>
  props.id !== undefined && !conversation.value && store.restoringLocalDrafts ? 'loading' : product.partLoad('conversations'),
)
const pageKind = computed(() =>
  !props.id && listStatus.value === 'ready'
    ? 'none'
    : conversationPageKind(listStatus.value, !!conversation.value),
)
// A new conversation's address, as a reload of it shows, has nothing to read.
watch(
  () => props.id,
  (id, previous) => {
    if (previous) {
      viewAbandoned(previous, null)
    }
    if (id) {
      viewOpened(id, null, previous === undefined ? 'load' : 'navigation')
    }
    void store.activate(id ?? null, { newConversation: navigation.showsNewConversation() })
  },
  { immediate: true },
)
onUnmounted(() => void store.activate(null))
// The chat's own address with no conversation open starts a new one, an
// immediately typeable draft, as New does. Settings opened by a link over that
// address leave it as it is, since moving the address would close them.
watch(
  () => router.currentRoute.value.path === '/chat' && listStatus.value === 'ready',
  (start) => {
    if (start) {
      navigation.replaceWithNew()
    }
  },
  { immediate: true },
)
// A conversation deleted while the page shows it, by this page or another,
// gives way to a new one. Under open settings the address waits until they close,
// since moving it would close them.
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
// it finds: a new conversation opens again. Any other shows that the
// conversation is not found.
watch(
  () => props.id !== undefined && !conversation.value && listStatus.value === 'ready' && router.currentRoute.value.path.startsWith('/chat/'),
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
  // The read wrote its own failure (`history-reads.ts`).
  read.catch(() => {})
}

// Once the first conversation has shown, what its regions need, the highlighter,
// KaTeX, the editor and the file icons' list, loads in the background.
watch(
  () => !!conversation.value && transcriptShown(conversation.value.load),
  (shown) => {
    if (shown)
      loadRegionRenderers()
  },
  { immediate: true, flush: 'post' },
)

// A search result opens at its message, and a report's title at its
// command's call: the page reads the window around it once the
// conversation's latest page is there, at the first step of its opening.
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
 * The Host files the conversation's messages name:
 *
 * - Images from the files the conversation's service keeps, at the version its
 *   file watch keeps current.
 * - Files opened through the `file` intent while a plugin opens it, at their
 *   lines.
 * - Commits opened through the `commit` intent, and a branch through the `branch` intent.
 * - The history a subagent's bar reads its branch from.
 * - Which paths of a reply's inline code are files, from the Host, whose home a
 *   `~/` path starts from.
 * - The attachments they deliver, a card's opened through the `attachment`
 *   intent while a plugin opens it.
 */
const files = computed<ConversationFiles | undefined>(() => {
  const current = conversation.value
  if (!current) {
    return undefined
  }
  const contents = conversationFiles(current.id).workspace?.source.contents
  const device = executionFor(current).deviceId
  return {
    home: resources.deviceById(device)?.home ?? null,
    ...(contents ? { contents } : {}),
    ...(device ? { device } : {}),
    open: work.canOpen('file')
      ? ({ path, line, root }) => work.openIn(current.id, { intent: 'file', payload: { path, ...(line === null ? {} : { line }), ...(root === undefined ? {} : { root }) } })
      : undefined,
    showInFiles: work.canOpen('file')
      ? ({ path, root }) => work.openIn(current.id, { intent: 'file', payload: { path, tree: true, ...(root === undefined ? {} : { root }) } })
      : undefined,
    openCommit: work.canOpen('commit')
      ? ({ commit, root }) => work.openIn(current.id, { intent: 'commit', payload: { commit, ...(root === undefined ? {} : { root }) } })
      : undefined,
    history: (root) => conversationFiles(current.id).history(root),
    openBranch: work.canOpen('branch')
      ? ({ root, branch }) => work.openIn(current.id, { intent: 'branch', payload: { root, branch } })
      : undefined,
    commitHash: async ({ commit, root }) => {
      const history = conversationFiles(current.id).history(root)
      if (!history) {
        throw new FileBrowserError('offline')
      }
      return readCommitHash(history, commit)
    },
    ...(attachments.value ? { attachment: attachments.value } : {}),
    attachmentOrigin: (id, attachment) => attachmentOrigin(id, attachment, {
      conversation: current.id,
      device: executionFor(current).deviceId ?? null,
      online: primaryHost.value?.state === 'online',
    }),
    openAttachment: work.canOpen('attachment')
      ? (id, { name, size, url }) => work.openIn(current.id, { intent: 'attachment', payload: { id, name, size, url } })
      : undefined,
  }
})

/** What a child's context card in the Agents window shows and opens beside its usage. */
const card = useContextCard(() => files.value?.open)

/** What the replies' comments do in the conversation's draft; an archived conversation takes none. */
const commenting = computed(() => (conversation.value ? store.commenting(conversation.value) : undefined))
/** The side questions on the replies; a conversation without a backend record has none yet. */
const asking = computed(() => {
  const shown = conversation.value
  return shown && shown.persistence === 'synced' && !shown.archived
    ? sideQuestions.asking(shown, (id, at) =>
        readHistory(at === 'latest' ? store.history.readLatest(shown, id) : store.history.readBefore(shown, id)))
    : undefined
})

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
    :catching-up="store.catchingUp(conversation)"
    :has-provider="hasProvider"
    :credential-kinds="credentialKinds"
    :is-exposed="exposedPorts.isExposed"
    :resume-at-retry="preferences.resumeAtRetry"
    @update:resume-at-retry="preferences.update({ resumeAtRetry: $event })"
    :fork="fork"
    :share="share"
    :searchable="conversation.persistence === 'synced'"
    :find-shortcut="resources.keys.find((binding) => binding.id === 'find')?.keys"
    @find="emit('find')"
    :select-edit="selectEdit"
    :file-pills="conversation.projectId !== null"
    :commenting="conversation.archived ? undefined : commenting"
    :asking="asking"
    :files="files"
    :primary-host="primaryHost"
    :permission-requests="permissions.stateFor(conversation.id).requests"
    :deciding-permission="permissions.stateFor(conversation.id).deciding"
    :request-error="permissions.stateFor(conversation.id).error"
    @decide-permission="(id, decision, credential) => permissions.decide(conversation!.id, id, decision, credential)"
    :pending-submissions="store.submissions(conversation)"
    :removing="conversation.removing"
    @retry-undelivered="store.retrySends(conversation)"
    @edit-submission="store.editSend(conversation, $event)"
    @delete-submission="store.deleteSend(conversation, $event)"
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
    @open-conversation="(at) => navigation.openAt(at.conversationId, at.node, at.blockId)"
    :read-block="async (node, id) => { await store.history.readWhole(conversation!, node, id) }"
    :reveal-command="(id) => revealCommand(conversation!, id)"
    @read-before="readHistory(store.history.readBefore(conversation!, null))"
    @read-after="readHistory(store.history.readAfter(conversation!, null))"
    @show-latest="store.history.showAt(conversation!, null, null)"
    :read-subagent-latest="(id) => store.history.readLatest(conversation!, id)"
    :subagents-streamed="store.streamsSubagents(conversation)"
    @read-subagent="(id) => readHistory(store.history.readBefore(conversation!, id))"
    @read-subagent-after="(id) => readHistory(store.history.readAfter(conversation!, id))"
    @shown="(node, block, within) => store.history.showAt(conversation!, node, block, within)"
    @show-subagent="store.showSubagent(conversation!, $event)"
    @open-aside="work.setOpen(conversation.id, true)"
    :usage-limits="card.limitsOf"
    @compact-subagent="store.compact(conversation, $event)"
    :subagent-cost="(id) => store.costShown(conversation!, id)"
    @refresh-limits="card.refreshLimits"
    @open-instruction="card.openInstruction"
    :schedules="schedules"
    @pause-schedule="(schedule, paused) => store.pauseSchedule(conversation!.id, schedule, paused)"
    @delete-schedule="(schedule) => store.deleteSchedule(conversation!.id, schedule)"
  >
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

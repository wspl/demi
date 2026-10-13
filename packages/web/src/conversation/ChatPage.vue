<script setup lang="ts">
import { computed, onUnmounted, useTemplateRef, watch } from 'vue'
import { useRouter } from 'vue-router'
import type { PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
import { conversationPageKind } from '@demicodes/web-ui/agent/session-status'
import ConversationComposer from './ConversationComposer.vue'
import { useContextCard } from './context-card'
import WorkspaceInfo from '../targets/WorkspaceInfo.vue'
import { executionFor, primaryHostOf } from '../targets/execution'
import { useConversations } from './store'
import { useConversationNavigation } from './navigation'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { useWorkPanel } from './work'
import { usePermissions } from './permissions'
import { conversationFileRoutes, rawFileContents } from '../api/files'
import type { EditSelectionHandler } from '@demicodes/web-ui/agent/edit-selection'
import { lookupAttachment } from '../api/attachments'
import { lookupCodeFiles } from '../api/code-files'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import type { MessageForkRequest } from '@demicodes/web-ui/agent/message-fork'
import type { Conversation } from '../state/types'

const store = useConversations()
const resources = useResources()
const product = useProduct()
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
// immediately typeable draft, as New does (`product.md` § Conversations and
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
// gives way to a new one (`product.md` § Conversations and projects). Under
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
watch(
  () => project.value?.id,
  (id) => {
    if (id) {
      resources.rememberProject(id)
    }
  },
  { immediate: true },
)
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
// conversation's latest page is there (`web-application.md` § Transcript
// windows).
watch(
  () => [store.reveal, conversation.value?.load] as const,
  ([reveal, load]) => {
    const current = conversation.value
    if (reveal && current && reveal.conversationId === current.id && load === 'ready') {
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

/**
 * The Host files the conversation's messages name: images from its raw
 * route, files opened through the `file` intent while a plugin opens it, at
 * their lines, and which paths of a reply's inline code are files, from the
 * Host, whose home a `~/` path starts from; and the attachments they name,
 * from the attachment route.
 */
const files = computed<ConversationFiles | undefined>(() => {
  const current = conversation.value
  if (!current) {
    return undefined
  }
  const contents = rawFileContents(conversationFileRoutes(current.id).raw)
  return {
    home: resources.deviceById(executionFor(current).deviceId)?.home ?? null,
    imageUrl: (path) => contents.url(path),
    open: work.canOpen('file')
      ? ({ path, line }) => work.openIn(current.id, { intent: 'file', payload: line === null ? { path } : { path, line } })
      : undefined,
    codeFiles: (paths) => lookupCodeFiles(current.id, paths, primaryHost.value?.state ?? 'offline'),
    attachment: (id) => lookupAttachment(current.id, id),
  }
})

/** What a child's context card in the Agents window shows and opens beside its usage. */
const card = useContextCard(() => files.value?.open)

async function fork(request: MessageForkRequest): Promise<void> {
  const sourceId = conversation.value?.id
  if (!sourceId) {
    throw new Error('The source conversation is unavailable.')
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
    :conversation="conversation"
    :has-provider="hasProvider"
    :backend-away="product.connection !== null"
    :fork="fork"
    :select-edit="selectEdit"
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
        error: pending.error,
      }))
    "
    :removing="conversation.removing"
    @retry-submission="store.retrySend(conversation, $event)"
    @retry="store.start(conversation)"
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
    @update:message-edit="conversation.messageEdit = $event"
    @regenerate="store.regenerate(conversation, $event)"
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

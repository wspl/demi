<script setup lang="ts">
import { computed, onUnmounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import type { PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
import { conversationPageKind } from '@demicodes/web-ui/agent/session-status'
import ConversationComposer from './ConversationComposer.vue'
import WorkspaceInfo from '../targets/WorkspaceInfo.vue'
import SessionTools from '../targets/SessionTools.vue'
import { useConversations } from './store'
import { useResources } from '../state/resources'
import { useWorkPanel } from './work'
import { conversationFileRoutes, rawFileContents } from '../api/files'
import type { EditSelectionHandler } from '@demicodes/web-ui/agent/edit-selection'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import type { MessageForkRequest } from '@demicodes/web-ui/agent/message-fork'

const store = useConversations()
const resources = useResources()
const work = useWorkPanel()
const route = useRoute()
const router = useRouter()
const conversation = computed(() =>
  store.items.find((c) => c.id === route.params.id),
)
const pageKind = computed(() =>
  !route.params.id && store.listStatus === 'ready'
    ? 'none'
    : conversationPageKind(store.listStatus, !!conversation.value),
)
watch(
  () => route.params.id,
  (id) => {
    void store.activate(typeof id === 'string' ? id : null)
  },
  { immediate: true },
)
onUnmounted(() => void store.activate(null))
async function create() {
  const id = await store.create()
  if (id) {
    await router.push(`/chat/${id}`)
  }
}
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

function saveScroll(id: string, state: PersistedScrollState | null): void {
  const item = store.items.find((item) => item.id === id)
  if (item) {
    item.scroll = state
  }
}

const selectEdit: EditSelectionHandler = (selection) => {
  const current = conversation.value
  if (current) {
    work.selectEdit(work.stateFor(current.id), selection)
  }
}

/** The Host files the conversation's messages name: images from its raw route, files opened in its work panel. */
const files = computed<ConversationFiles | undefined>(() => {
  const current = conversation.value
  if (!current) {
    return undefined
  }
  const contents = rawFileContents(conversationFileRoutes(current.id).raw)
  const state = work.stateFor(current.id)
  return {
    imageUrl: (path) => contents.url(path),
    open: (path) => work.open(state, path),
  }
})

async function fork(request: MessageForkRequest): Promise<void> {
  const sourceId = conversation.value?.id
  if (!sourceId) {
    throw new Error('The source conversation is unavailable')
  }
  const id = await store.fork(sourceId, request)
  if (route.params.id === sourceId) {
    await router.push(`/chat/${id}`)
  }
}
</script>

<template>
  <ChatSession
    v-if="pageKind === 'session' && conversation"
    :conversation="conversation"
    :has-provider="hasProvider"
    :fork="fork"
    :select-edit="selectEdit"
    :files="files"
    :pending-submission="
      conversation.pendingSend
        ? {
            id: conversation.pendingSend.id,
            text: conversation.pendingSend.text,
            attachments: conversation.pendingSend.fileIds.flatMap((id) =>
              conversation?.files.filter((file) => file.id === id) ?? [],
            ),
            error: conversation.pendingSend.error,
            sending: conversation.submission === 'sending',
          }
        : null
    "
    @retry-submission="store.send(conversation)"
    @retry="store.start(conversation)"
    @rename="store.rename(conversation.id, $event)"
    @retry-load="store.reloadSession(conversation.id)"
    @abort-subagents="store.abortSubagents(conversation)"
    @abort-subagent="store.abortSubagent(conversation, $event)"
    @abort-terminal="store.abortTerminal(conversation, $event)"
    @remove-queued="store.removeQueued(conversation, $event)"
    @send-queued="store.sendQueued(conversation, $event)"
    @remove-pending-steer="store.removePendingSteer(conversation, $event)"
    @interrupt-pending-steer="store.interruptWithSteer(conversation, $event)"
    :edit-version="store.editVersion(conversation)"
    :message-edit="conversation.messageEdit"
    :aside-open="work.stateFor(conversation.id).open"
    @update:message-edit="conversation.messageEdit = $event"
    @save-scroll="saveScroll"
    @open-aside="work.setOpen(work.stateFor(conversation.id), true)"
  >
    <template #workspace
      ><WorkspaceInfo :project="project" :conversation="conversation"
    /></template>
    <template #tools><SessionTools :conversation-id="conversation.id" /></template>
    <template #composer
      ><ConversationComposer
        :key="conversation.id"
        :conversation="conversation"
    /></template>
  </ChatSession>
  <section
    v-else
    class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-tl-xl bg-surface"
  >
    <SessionStatus
      :kind="pageKind === 'session' ? 'missing' : pageKind"
      @retry="store.reloadList()"
      @create="create"
    />
  </section>
</template>
